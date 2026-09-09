import { routerHook, toaster } from "@decky/api";
import { useEffect } from "react";

import type { Game } from "./api-types";
import {
  checkCloudSaveStatus,
  getLibrary,
  logEvent,
  restoreCloudSave,
} from "./events";
import {
  useAuthStore,
  useCloudSaveGuard,
  useCurrentGame,
  useLibraryStore,
  useSyncSettings,
  useUserStore,
} from "./stores";
import { composeToastLogo } from "./helpers";

// In-flight sync/restore operations, keyed by objectId. The launch/exit
// handler awaits these instead of racing them; entries delete on settle.
const busy = new Map<string, Promise<unknown>>();

// Session caches, invalidated on game exit, guard-flag change, and manual
// sync/restore.
const verifiedThisSession = new Set<string>();
const notifiedThisSession = new Set<string>();

export const invalidatePrePlayCache = (objectId: string) => {
  verifiedThisSession.delete(objectId);
  notifiedThisSession.delete(objectId);
};

export const trackBusy = <T,>(objectId: string, work: Promise<T>): Promise<T> => {
  busy.set(objectId, work);
  work.finally(() => {
    if (busy.get(objectId) === work) busy.delete(objectId);
  });
  return work;
};

export const waitForBusy = (objectId: string): Promise<unknown> | undefined =>
  busy.get(objectId)?.catch(() => {});

const findGameByShortcut = (appId: string): Game | undefined =>
  useLibraryStore
    .getState()
    .library.find(
      (game) => String(game.steamShortcutAppId ?? "") === String(appId)
    );

const onGamePageOpen = async (appId: string) => {
  if (!useSyncSettings.getState().syncBeforePlay) return;

  let game = findGameByShortcut(appId);
  if (!game) {
    // Shortcut may have been recreated since the last library load; refresh
    // once before giving up.
    useLibraryStore.getState().setLibrary(await getLibrary());
    game = findGameByShortcut(appId);
    if (!game) return;
  }

  if (!game.automaticCloudSync) return;
  if (verifiedThisSession.has(game.objectId)) return;
  if (useCurrentGame.getState().objectId === game.objectId) return;

  const { auth } = useAuthStore.getState();
  const { hasActiveSubscription } = useUserStore.getState();
  if (!auth || !hasActiveSubscription) return;

  if (busy.has(game.objectId)) {
    await waitForBusy(game.objectId);
    return;
  }

  const work = (async () => {
    const status = await checkCloudSaveStatus(auth, game.objectId, game.winePrefixPath);
    if (status.auth) useAuthStore.getState().setAuth(status.auth);

    if (!status.remoteNewer) {
      verifiedThisSession.add(game.objectId);
      return;
    }

    if (!status.localDirty) {
      // Local side is clean (or absent): restoring cannot lose progress.
      toaster.toast({
        title: "Syncing cloud save...",
        body: `${game.title} has a newer save in the cloud; restoring it now.`,
        logo: composeToastLogo(game.iconUrl),
      });
      logEvent(`auto-restore start: ${game.objectId} (remote v${status.remoteVersion})`);

      try {
        const result = await trackBusy(
          game.objectId,
          restoreCloudSave(auth, game.objectId, game.winePrefixPath)
        );
        if (result.auth) useAuthStore.getState().setAuth(result.auth);

        if (result.skippedFiles.length === 0) {
          useCloudSaveGuard.getState().clearRemoteNewer(game.objectId);
          verifiedThisSession.add(game.objectId);
          toaster.toast({
            title: "Cloud save restored",
            body: `${game.title} save is up to date (${result.restoredFiles} files).`,
            logo: composeToastLogo(game.iconUrl),
          });
        } else {
          toaster.toast({
            title: "Cloud save partially restored",
            body: `${game.title}: ${result.skippedFiles.length} files could not be restored.`,
            logo: composeToastLogo(game.iconUrl),
          });
        }
        logEvent(`auto-restore done: ${game.objectId} v${result.version}`);
      } catch (error) {
        console.error("Pre-play restore failed", error);
        logEvent(`auto-restore failed: ${game.objectId}: ${error instanceof Error ? error.message : "unknown"}`);
        toaster.toast({
          title: "Cloud save sync failed",
          body: `Could not restore the cloud save for ${game.title}. Your local save was not modified.`,
        });
      }
      return;
    }

    // Local progress would be lost by an automatic restore: notify once and
    // let the user resolve it from the plugin.
    useCloudSaveGuard.getState().flagRemoteNewer(game.objectId);
    if (!notifiedThisSession.has(game.objectId)) {
      notifiedThisSession.add(game.objectId);
      logEvent(`pre-play conflict notice: ${game.objectId}`);
      toaster.toast({
        title: "Save sync needs a decision",
        body: `${game.title} has save changes both locally and in the cloud. Open the Hydra plugin to choose which to keep.`,
        logo: composeToastLogo(game.iconUrl),
      });
    }
  })();

  busy.set(game.objectId, work);
  work.finally(() => {
    if (busy.get(game.objectId) === work) busy.delete(game.objectId);
  });
  await work.catch(() => {});
};

const AppPageSync = ({ appid }: { appid?: string }) => {
  useEffect(() => {
    if (appid) onGamePageOpen(String(appid));
  }, [appid]);
  return null;
};

let patchHandle: ((route: any) => any) | null = null;

export const registerPrePlaySync = () => {
  const patch = (route: any) => {
    const Original = route.component;
    return {
      ...route,
      component: function WrappedAppPage(props: any) {
        return (
          <>
            <AppPageSync appid={props?.match?.params?.appid} />
            {Original ? <Original {...props} /> : null}
          </>
        );
      },
    };
  };
  patchHandle = patch;
  routerHook.addPatch("/library/app/:appid", patch as any);
};

export const unregisterPrePlaySync = () => {
  if (patchHandle) {
    routerHook.removePatch("/library/app/:appid", patchHandle as any);
    patchHandle = null;
  }
};
