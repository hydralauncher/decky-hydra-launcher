import { routerHook, toaster, type RoutePatch } from "@decky/api";
import { useEffect, type ReactNode } from "react";

import type { Game } from "./api-types";
import {
  checkCloudSaveStatus,
  getLibrary,
  isHydraLauncherRunning,
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
  useSyncStatusStore,
  useUserStore,
} from "./stores";
import {
  disengagePlayBlock,
  engagePlayBlock,
  setActiveAppPage,
} from "./play-block";
import { composeToastLogo } from "./helpers";
import { PiCloudArrowDown } from "react-icons/pi";
import { showSyncToast, SYNC_TOAST_TIMEOUT_MS, SyncToastBody } from "./sync-toast";
import { SyncBlockOverlay } from "./sync-block-overlay";

const busy = new Map<string, ActiveOperation>();

const shortcutResolved = new Map<string, string | null>();

const verifiedAt = new Map<string, number>();
const notifiedThisSession = new Set<string>();
const hydraRunningToastShown = new Set<string>();

const VERIFIED_TTL_MS = 30_000;

const BUSY_ATTACH_TIMEOUT_MS = 30_000;

const isRecentlyVerified = (objectId: string): boolean => {
  const at = verifiedAt.get(objectId);
  return at !== undefined && Date.now() - at < VERIFIED_TTL_MS;
};

const skipLoggedThisSession = new Set<string>();

const logSkipOnce = (key: string, message: string) => {
  if (skipLoggedThisSession.has(key)) return;
  skipLoggedThisSession.add(key);
  logEvent(`pre-play skip: ${message}`);
};

export const invalidatePrePlayCache = (objectId: string) => {
  verifiedAt.delete(objectId);
  notifiedThisSession.delete(objectId);
  for (const key of [...skipLoggedThisSession]) {
    if (key.startsWith(`${objectId}:`)) skipLoggedThisSession.delete(key);
  }
};

export const markPrePlayVerified = (objectId: string) => {
  verifiedAt.set(objectId, Date.now());
};

export type SyncOperationKey = "sync" | "restore";

interface ActiveOperation {
  key: SyncOperationKey;
  promise: Promise<unknown>;
}

export const trackBusy = <T,>(
  objectId: string,
  key: SyncOperationKey,
  start: () => Promise<T>
): Promise<T> => {
  const run = (): Promise<T> => {
    const promise = start();
    const entry: ActiveOperation = { key, promise: promise as Promise<unknown> };
    busy.set(objectId, entry);
    const settle = () => {
      if (busy.get(objectId) === entry) busy.delete(objectId);
    };
    promise.then(settle, settle);
    return promise;
  };
  const active = busy.get(objectId);
  if (!active) return run();
  if (active.key === key) return active.promise as Promise<T>;
  logEvent(`sync op queued: ${objectId} ${key} behind ${active.key}`);
  return active.promise.then(run, run);
};

export const waitForBusy = (
  objectId: string,
  timeoutMs: number = BUSY_ATTACH_TIMEOUT_MS
): Promise<unknown> => {
  const active = busy.get(objectId);
  if (!active) return Promise.resolve();
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(undefined), timeoutMs);
    active.promise.then(
      () => {
        clearTimeout(timer);
        resolve(undefined);
      },
      () => {
        clearTimeout(timer);
        resolve(undefined);
      }
    );
  });
};

export const isOperationActive = (objectId: string): boolean =>
  busy.has(objectId);

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

  try {
    if (await isHydraLauncherRunning()) {
      logSkipOnce(`page:${appId}:hydra-running`, `hydra launcher running (page appid ${appId})`);
      if (!hydraRunningToastShown.has(String(appId))) {
        hydraRunningToastShown.add(String(appId));
        toaster.toast({
            title: "Desktop launcher active",
            body: "Deck sync is paused while the desktop launcher runs.",
        });
      }
      return;
    }
  } catch (error) {
    logEvent(`pre-play launcher check failed: ${error instanceof Error ? error.message : "unknown"}`);
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
  if (isRecentlyVerified(game.objectId)) return;
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

  engagePlayBlock(game.objectId);

  const work = (async () => {
    let status;
    useSyncStatusStore.getState().setChecking(game.objectId, true);
    try {
      status = await checkCloudSaveStatus(auth, game.objectId, game.shop, game.winePrefixPath);
    } catch (error) {
      logEvent(`pre-play status failed: ${game.objectId}: ${error instanceof Error ? error.message : "unknown"}`);
      useSyncStatusStore.getState().setChecking(game.objectId, false);
      disengagePlayBlock(game.objectId);
      return;
    }
    useSyncStatusStore.getState().setChecking(game.objectId, false);
    if (status.auth) useAuthStore.getState().setAuth(status.auth);
    useSyncStatusStore.getState().setStatus(game.objectId, {
      remoteVersion: status.remoteVersion ?? null,
      localVersion: status.localVersion ?? null,
      remoteFileCount: status.remoteFileCount ?? null,
      remoteTotalBytes: status.remoteTotalBytes ?? null,
      remoteUpdatedAt: status.remoteUpdatedAt ?? null,
      localFileCount: status.localFileCount ?? null,
      localTotalBytes: status.localTotalBytes ?? null,
      localUpdatedAt: status.localUpdatedAt ?? null,
    });

    if (!status.remoteNewer) {
      markPrePlayVerified(game.objectId);
      logEvent(`pre-play verified: ${game.objectId} (remote v${status.remoteVersion ?? "none"})`);
      disengagePlayBlock(game.objectId);
      return;
    }

    if (!status.localDirty) {
      logEvent(`auto-restore start: ${game.objectId} (remote v${status.remoteVersion})`);
      showSyncToast(game.objectId, {
        title: game.title,
        body: <SyncToastBody text="Syncing cloud save…" />,
        logo: composeToastLogo(game.iconUrl),
        duration: SYNC_TOAST_TIMEOUT_MS,
      });

      try {
        const result = await trackBusy(
          game.objectId,
          "restore",
          () => restoreCloudSave(auth, game.objectId, game.shop, game.winePrefixPath)
        );
        if (result.auth) useAuthStore.getState().setAuth(result.auth);

        if (result.skippedFiles.length === 0) {
          useCloudSaveGuard.getState().clearRemoteNewer(game.objectId);
          markPrePlayVerified(game.objectId);
          toaster.toast({
            title: "Cloud save restored",
            body: `${game.title} is up to date (${result.restoredFiles} files).`,
            logo: composeToastLogo(game.iconUrl),
            icon: <PiCloudArrowDown size={20} />,
          });
        } else {
          toaster.toast({
            title: "Cloud save partially restored",
            body: `${game.title}: couldn't restore ${result.skippedFiles.length} files.`,
            logo: composeToastLogo(game.iconUrl),
            icon: <PiCloudArrowDown size={20} />,
          });
        }
        logEvent(`auto-restore done: ${game.objectId} v${result.version}`);
      } catch (error) {
        console.error("Pre-play restore failed", error);
        logEvent(`auto-restore failed: ${game.objectId}: ${error instanceof Error ? error.message : "unknown"}`);
        toaster.toast({
          title: "Cloud save sync failed",
          body: `Restore failed for ${game.title}. Local save untouched.`,
        });
      } finally {
        disengagePlayBlock(game.objectId);
      }
      return;
    }

    useCloudSaveGuard.getState().flagRemoteNewer(game.objectId);
    if (!notifiedThisSession.has(game.objectId)) {
      notifiedThisSession.add(game.objectId);
      logEvent(`pre-play conflict notice: ${game.objectId}`);
      toaster.toast({
        title: "Save sync needs a decision",
        body: `${game.title}: changed on both sides. Choose in Hydra → Pending decisions.`,
      });
    }
  })();

  await work.catch(() => {});
};

const AppPageSync = ({ appid }: { appid?: string }) => {
  const routeAppId =
    window.location.pathname.match(/\/library\/app\/(\d+)/)?.[1] ??
    window.location.hash.match(/\/library\/app\/(\d+)/)?.[1];
  const effectiveAppId = appid ?? routeAppId;

  useEffect(() => {
    setActiveAppPage(effectiveAppId ? String(effectiveAppId) : null);
    logEvent(
      `pre-play page mount: appid=${effectiveAppId ?? "none"} path=${window.location.pathname} hash=${window.location.hash}`
    );
    return () => setActiveAppPage(null);
  }, [effectiveAppId]);

  useEffect(() => {
    if (effectiveAppId) onGamePageOpen(String(effectiveAppId));
  }, [effectiveAppId]);
  return effectiveAppId ? <SyncBlockOverlay appid={String(effectiveAppId)} /> : null;
};

let patchHandle: RoutePatch | null = null;

useCloudSaveGuard.subscribe((state, prev) => {
  for (const id of prev.remoteNewerGames) {
    if (!state.remoteNewerGames.includes(id)) {
      disengagePlayBlock(id);
    }
  }
});

const WRAPPED = "__hydraPrePlayWrapped";

interface AppPageRouteProps {
  match?: { params?: { appid?: string } } | null;
}

interface PatchableRoute {
  children?: ReactNode | ((props: AppPageRouteProps) => ReactNode);
  component?: unknown;
  render?: unknown;
  renderFunc?: unknown;
  element?: unknown;
}

function withSyncTracking(rendered: ReactNode, props: AppPageRouteProps) {
  return (
    <>
      <AppPageSync appid={props?.match?.params?.appid} />
      {rendered}
    </>
  );
}

function routeShape(route: PatchableRoute | null): string {
  if (!route) return "null";
  if (route.component) return "component";
  if (typeof route.render === "function") return "render";
  if (typeof route.renderFunc === "function") return "renderFunc";
  if (route.element !== undefined && route.element !== null) return "element";
  return "none:" + Object.keys(route).join(",");
}

export const registerPrePlaySync = () => {
  const patch: RoutePatch = (route) => {
    const target = route as PatchableRoute | null;
    if (!target || (target as Record<string, unknown>)[WRAPPED]) return route;
    logEvent(`pre-play patch applied: shape=${routeShape(target)}`);
    if (target.children !== undefined && target.children !== null) {
      const children = target.children;
      return {
        ...target,
        [WRAPPED]: true,
        children:
          typeof children === "function"
            ? (props: AppPageRouteProps) => withSyncTracking(children(props), props)
            : withSyncTracking(children, {}),
      };
    }
    return route;
  };
  patchHandle = patch;
  routerHook.addPatch("/library/app/:appid", patch);
};

export const unregisterPrePlaySync = () => {
  if (patchHandle) {
    routerHook.removePatch("/library/app/:appid", patchHandle);
    patchHandle = null;
  }
};
