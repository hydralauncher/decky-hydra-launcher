import { routerHook, toaster } from "@decky/api";
import { useEffect, type ReactNode } from "react";

import type { Game } from "./api-types";
import {
  checkCloudSaveStatus,
  getLibrary,
  logEvent,
  resolveShortcut,
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
import {
  disengagePlayBlock,
  engagePlayBlock,
  setActiveAppPage,
} from "./play-block";
import { composeToastLogo } from "./helpers";

// In-flight sync/restore operations, keyed by objectId. The launch/exit
// handler awaits these instead of racing them; entries delete on settle.
const busy = new Map<string, Promise<unknown>>();

const shortcutResolved = new Map<string, string | null>();

// Session caches, invalidated on game exit, guard-flag change, and manual
// sync/restore.
const verifiedThisSession = new Set<string>();
const notifiedThisSession = new Set<string>();

const skipLoggedThisSession = new Set<string>();

const logSkipOnce = (key: string, message: string) => {
  if (skipLoggedThisSession.has(key)) return;
  skipLoggedThisSession.add(key);
  logEvent(`pre-play skip: ${message}`);
};

export const invalidatePrePlayCache = (objectId: string) => {
  verifiedThisSession.delete(objectId);
  notifiedThisSession.delete(objectId);
  for (const key of [...skipLoggedThisSession]) {
    if (key.startsWith(`${objectId}:`)) skipLoggedThisSession.delete(key);
  }
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

export const findGameByShortcutId = (appId: string): Game | undefined => {
  const key = String(appId);
  const library = useLibraryStore.getState().library;
  const direct = library.find(
    (game) => String(game.steamShortcutAppId ?? "") === key
  );
  if (direct) return direct;
  const objectId = shortcutResolved.get(key);
  if (!objectId) return undefined;
  return library.find((game) => game.objectId === objectId);
};

const onGamePageOpen = async (appId: string) => {
  if (!useSyncSettings.getState().syncBeforePlay) {
    logSkipOnce(`page:${appId}:toggle-off`, `toggle off (page appid ${appId})`);
    return;
  }

  let game = findGameByShortcutId(appId);
  if (!game && !shortcutResolved.has(String(appId))) {
    try {
      const resolved = await resolveShortcut(String(appId));
      shortcutResolved.set(String(appId), resolved?.objectId ?? null);
      if (resolved) {
        logSkipOnce(
          `${resolved.objectId}:match-${resolved.source}`,
          `shortcut match via ${resolved.source} (${resolved.objectId})`
        );
      }
    } catch (error) {
      console.error("Shortcut resolve failed", error);
    }
    game = findGameByShortcutId(appId);
  }
  if (!game) {
    const mapped = shortcutResolved.get(String(appId));
    const libraryMissing =
      mapped != null &&
      !useLibraryStore.getState().library.some((g) => g.objectId === mapped);
    if (libraryMissing || !shortcutResolved.has(String(appId))) {
      useLibraryStore.getState().setLibrary(await getLibrary());
      game = findGameByShortcutId(appId);
    }
    if (!game) {
      logSkipOnce(`page:${appId}:miss`, `no shortcut match for appid ${appId}`);
      return;
    }
  }

  if (!game.automaticCloudSync) {
    logSkipOnce(`${game.objectId}:auto-sync-off`, `auto-sync off (${game.objectId})`);
    return;
  }
  if (verifiedThisSession.has(game.objectId)) return;
  if (useCurrentGame.getState().objectId === game.objectId) {
    logSkipOnce(`${game.objectId}:running`, `game running (${game.objectId})`);
    return;
  }

  const { auth } = useAuthStore.getState();
  const { hasActiveSubscription } = useUserStore.getState();
  if (!auth) {
    logSkipOnce(`${game.objectId}:no-auth`, `no auth (${game.objectId})`);
    return;
  }
  if (!hasActiveSubscription) {
    logSkipOnce(`${game.objectId}:no-sub`, `no subscription (${game.objectId})`);
    return;
  }

  if (busy.has(game.objectId)) {
    logSkipOnce(`${game.objectId}:busy`, `attach in-flight (${game.objectId})`);
    await waitForBusy(game.objectId);
    return;
  }

  const work = (async () => {
    const status = await checkCloudSaveStatus(auth, game.objectId, game.winePrefixPath);
    if (status.auth) useAuthStore.getState().setAuth(status.auth);

    if (!status.remoteNewer) {
      verifiedThisSession.add(game.objectId);
      logEvent(`pre-play verified: ${game.objectId} (remote v${status.remoteVersion ?? "none"})`);
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
      engagePlayBlock(game.objectId);

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
      } finally {
        disengagePlayBlock(game.objectId);
      }
      return;
    }

    // Local progress would be lost by an automatic restore: notify once and
    // let the user resolve it from the plugin.
    useCloudSaveGuard.getState().flagRemoteNewer(game.objectId);
    engagePlayBlock(game.objectId);
    if (!notifiedThisSession.has(game.objectId)) {
      notifiedThisSession.add(game.objectId);
      logEvent(`pre-play conflict notice: ${game.objectId}`);
      toaster.toast({
        title: "Save sync needs a decision",
        body: `${game.title} has save changes both locally and in the cloud. Open the Hydra plugin → Pending decisions to choose which to keep.`,
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
  // Some route shapes (children/element) forward no props; parse the hash
  // route as fallback (Steam uses a hash router: #/library/app/<appid>).
  const hashAppId =
    window.location.hash.match(/\/library\/app\/(\d+)/)?.[1];
  const effectiveAppId = appid ?? hashAppId;

  useEffect(() => {
    setActiveAppPage(effectiveAppId ? String(effectiveAppId) : null);
    logEvent(`pre-play page mount: appid=${effectiveAppId ?? "none"}`);
    return () => setActiveAppPage(null);
  }, [effectiveAppId]);

  useEffect(() => {
    if (effectiveAppId) onGamePageOpen(String(effectiveAppId));
  }, [effectiveAppId]);
  return null;
};

let patchHandle: ((route: any) => any) | null = null;

// Guard resolution (manual sync/restore) releases any block.
useCloudSaveGuard.subscribe((state, prev) => {
  for (const id of prev.remoteNewerGames) {
    if (!state.remoteNewerGames.includes(id)) {
      disengagePlayBlock(id);
    }
  }
});

const WRAPPED = "__hydraPrePlayWrapped";

function withSyncTracking(rendered: ReactNode, props: any) {
  return (
    <>
      <AppPageSync appid={props?.match?.params?.appid} />
      {rendered}
    </>
  );
}

function routeShape(route: any): string {
  if (!route) return "null";
  if (route.component) return "component";
  if (typeof route.render === "function") return "render";
  if (typeof route.renderFunc === "function") return "renderFunc";
  if (route.element !== undefined && route.element !== null) return "element";
  return "none:" + Object.keys(route).join(",");
}

export const registerPrePlaySync = () => {
  const patch = (route: any) => {
    if (!route || route[WRAPPED]) return route;
    logEvent(`pre-play patch applied: shape=${routeShape(route)}`);
    if (route.component) {
      const Original = route.component;
      return {
        ...route,
        [WRAPPED]: true,
        component: (props: any) =>
          withSyncTracking(Original ? <Original {...props} /> : null, props),
      };
    }
    if (typeof route.render === "function") {
      const fn = route.render;
      return {
        ...route,
        [WRAPPED]: true,
        render: (props: any) => withSyncTracking(fn(props), props),
      };
    }
    if (typeof route.renderFunc === "function") {
      const fn = route.renderFunc;
      return {
        ...route,
        [WRAPPED]: true,
        renderFunc: (props: any) => withSyncTracking(fn(props), props),
      };
    }
    if (route.element !== undefined && route.element !== null) {
      return {
        ...route,
        [WRAPPED]: true,
        element: withSyncTracking(route.element, {}),
      };
    }
    if (route.children !== undefined && route.children !== null) {
      const children = route.children;
      return {
        ...route,
        [WRAPPED]: true,
        children:
          typeof children === "function"
            ? (props: any) => withSyncTracking(children(props), props)
            : withSyncTracking(children, {}),
      };
    }
    return route;
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
