import { staticClasses } from "@decky/ui";
import { definePlugin, toaster } from "@decky/api";
import { useEffect, useMemo } from "react";
import { AppLifetimeNotification } from "@decky/ui/dist/globals/steam-client/GameSessions";
import styles from "./styles/globals.scss";
import {
  useAuthStore,
  useCloudSaveGuard,
  useCurrentGame,
  useLibraryStore,
  useNavigationStore,
  useUserStore,
} from "./stores";
import { api } from "./hydra-api";
import { Home } from "./home";
import { WSClient } from "./ws";
import { composeToastLogo } from "./helpers";
import { PiCloudArrowUp } from "react-icons/pi";
import { GameCloudSaves } from "./game-cloud-saves";
import { AuthGuide } from "./auth-guide";
import {
  checkCloudSaveStatus,
  getAuth,
  getLibrary,
  isHydraLauncherRunning,
  logEvent,
  syncCloudSave,
} from "./events";
import {
  clearShortcutNegativeCache,
  hasShortcutResolution,
  invalidatePrePlayCache,
  isRemoteNewerError,
  registerPrePlaySync,
  resolveGameForAppId,
  trackBusy,
  unregisterPrePlaySync,
  waitForBusy,
} from "./pre-play-sync";
import {
  disengagePlayBlock,
  engagePlayBlock,
  registerPlayBlock,
  unregisterPlayBlock,
} from "./play-block";
import { HydraLogo } from "./components";
import type { Game, User } from "./api-types";

function Plugin() {
  const { route, setRoute } = useNavigationStore();
  const { auth } = useAuthStore();

  useEffect(() => {
    if (!auth) {
      setRoute({
        name: "auth-guide",
        params: {},
      });
    } else {
      setRoute({
        name: "home",
        params: {},
      });
    }
  }, [auth, setRoute]);

  const content = useMemo(() => {
    switch (route?.name) {
      case "auth-guide":
        return <AuthGuide />;
      case "game":
        return <GameCloudSaves key={(route.params.game as Game).objectId} game={route.params.game as Game} />;
      case "home":
        return <Home />;
      default:
        return null;
    }
  }, [route]);

  return (
    <>
      <style>{styles}</style>

      {content}
    </>
  );
}

let updateInterval: NodeJS.Timeout;
let lastTick: Date;

const PLAYTIME_TICK_SECONDS = 10;

const pendingStatusChecks = new Map<string, Promise<void>>();

const sessionState = new Map<string, { status: "active" | "exited"; at: number }>();

const recordSessionStart = (objectId: string) => {
  const cutoff = Date.now() - 24 * 60 * 60 * 1000;
  for (const [key, value] of [...sessionState]) {
    if (value.status === "exited" && value.at < cutoff) sessionState.delete(key);
  }
  sessionState.set(objectId, { status: "active", at: Date.now() });
  while (sessionState.size > 50) {
    const oldest = [...sessionState.entries()].sort((a, b) => a[1].at - b[1].at)[0]?.[0];
    if (!oldest) break;
    sessionState.delete(oldest);
  }
};

const consumeSessionExit = (objectId: string): "active" | "exited" | "unknown" => {
  const entry = sessionState.get(objectId);
  if (!entry) return "unknown";
  if (entry.status === "exited") return "exited";
  sessionState.set(objectId, { status: "exited", at: Date.now() });
  return "active";
};

const lifetimeChains = new Map<string, Promise<void>>();

let playtimeOwner: string | null = null;
let notifySeq = 0;
let latestLaunchSeq = 0;

const handleAppLifetimeNotification = async (
  notification: AppLifetimeNotification,
  seq: number
) => {
  const {
    clearGame,
    setStartedAt,
    setObjectId,
    setRemoteId,
    setElapsedTimeInMillis,
  } = useCurrentGame.getState();
  const { setLibrary } = useLibraryStore.getState();
  const { auth } = useAuthStore.getState();
  const { hasActiveSubscription } = useUserStore.getState();

  const library = await getLibrary();
  setLibrary(library);
  const clearedNegatives = clearShortcutNegativeCache();
  if (clearedNegatives > 0) {
    logEvent(`shortcut cache: cleared ${clearedNegatives} negatives`);
  }

  const unAppID = notification.unAppID.toString();

  const libraryGame = library.find((item) => {
    return (
      item.objectId === unAppID ||
      String(item.steamShortcutAppId ?? "") === unAppID
    );
  });
  const cloudGame = await resolveGameForAppId(unAppID);
  const playGame = cloudGame ?? libraryGame;

  logEvent(
    `app lifetime: unAppID=${unAppID} running=${notification.bRunning} library=${libraryGame ? libraryGame.objectId : "none"} eligible=${cloudGame ? cloudGame.objectId : "none"}`
  );

  if (!playGame) {
    logEvent(`lifetime skip: no library match for appid ${unAppID}`);
    return;
  }

  if (!cloudGame && libraryGame && hasShortcutResolution(unAppID)) {
    const guard = useCloudSaveGuard.getState();
    if (guard.remoteNewerGames.includes(libraryGame.objectId)) {
      guard.clearRemoteNewer(libraryGame.objectId);
      logEvent(`guard prune: ineligible on sight (${libraryGame.objectId})`);
    }
  }

  logEvent(
    `app ${notification.bRunning ? "launch" : "exit"}: ${playGame.title} (${playGame.objectId})`
  );

    if (notification.bRunning) {
      if (seq !== latestLaunchSeq) {
        logEvent(`launch superseded (${playGame.objectId})`);
        return;
      }
      if (updateInterval) {
        clearInterval(updateInterval);
      }
      clearGame();
      playtimeOwner = playGame.objectId;
      const startedAt = new Date();
      lastTick = startedAt;

      setObjectId(playGame.objectId);
      setRemoteId(playGame.remoteId);
      setStartedAt(startedAt);

      if (!cloudGame) {
        logEvent(`cloud skip: ineligible (${playGame.objectId})`);
      }

      if (cloudGame) {
        recordSessionStart(cloudGame.objectId);
        disengagePlayBlock(cloudGame.objectId);

        const hydraAtLaunch = await isHydraLauncherRunning();
        if (hydraAtLaunch) {
          logEvent(`launch status skipped: hydra running (${cloudGame.objectId})`);
        }

        const alreadyFlagged = useCloudSaveGuard
          .getState()
          .remoteNewerGames.includes(cloudGame.objectId);

        if (!hydraAtLaunch && cloudGame.automaticCloudSync && auth && hasActiveSubscription && !alreadyFlagged) {
          await waitForBusy(cloudGame.objectId);

          const check = checkCloudSaveStatus(auth, cloudGame.objectId, cloudGame.shop, cloudGame.winePrefixPath)
            .then((status) => {
              if (status.auth) {
                useAuthStore.getState().setAuth(status.auth);
              }
              if (status.remoteNewer) {
                useCloudSaveGuard.getState().flagRemoteNewer(cloudGame.objectId);
                logEvent(`guard flagged: ${cloudGame.objectId} (remote v${status.remoteVersion}, local v${status.localVersion ?? "none"})`);
                if (useCurrentGame.getState().objectId === cloudGame.objectId) {
                  toaster.toast({
                    title: "Newer cloud save available",
                    body: `${cloudGame.title}: newer save in the cloud. This session won't sync — restore it in Hydra.`,
                    logo: composeToastLogo(cloudGame.iconUrl),
                  });
                }
              } else {
                useCloudSaveGuard.getState().clearRemoteNewer(cloudGame.objectId);
                logEvent(`guard clear: ${cloudGame.objectId} (remote v${status.remoteVersion})`);
              }
            })
            .catch((err) => {
              console.error("Failed to check cloud save status", err);
              const message = err instanceof Error ? err.message : "unknown";
              logEvent(`launch status failed: ${cloudGame.objectId}: ${message}`);
              if (isRemoteNewerError(err)) {
                useCloudSaveGuard.getState().flagRemoteNewer(cloudGame.objectId);
              }
              toaster.toast({
                title: "Cloud save status unknown",
                body: `Couldn't check ${cloudGame.title}. Exit sync still runs at close.`,
              });
            })
            .finally(() => {
              if (pendingStatusChecks.get(cloudGame.objectId) === check) {
                pendingStatusChecks.delete(cloudGame.objectId);
              }
            });

          pendingStatusChecks.set(cloudGame.objectId, check);
        }
      }

      updateInterval = setInterval(async () => {
        const secondsSinceLastTick = Math.floor(
          (new Date().getTime() - lastTick.getTime()) / 1_000
        );

        setElapsedTimeInMillis(Date.now() - startedAt.getTime());

        if (secondsSinceLastTick >= PLAYTIME_TICK_SECONDS) {
          const isHydraRunning = await isHydraLauncherRunning();

          if (isHydraRunning) {
            return;
          }

          lastTick = new Date();

          api
            .put(`profile/games/${playGame.remoteId}`, {
              json: {
                playTimeDeltaInSeconds: secondsSinceLastTick,
                lastTimePlayed: startedAt,
              },
            })
            .catch((err) => {
              console.error("Failed to update playtime", err);
            });
        }
      }, 1_000);

      return;
    }

    if (playtimeOwner !== null && playtimeOwner !== playGame.objectId) {
      logEvent(`exit preserves active session (${playGame.objectId} keeps ${playtimeOwner})`);
    } else {
      if (updateInterval) {
        clearInterval(updateInterval);
      }
      clearGame();
      playtimeOwner = null;
    }

    if (!cloudGame) {
      logEvent(`auto-sync skipped: ineligible (${playGame.objectId})`);
      return;
    }

    const session = consumeSessionExit(cloudGame.objectId);
    if (session === "exited") {
      logEvent(`auto-sync skipped: duplicate exit (${cloudGame.objectId})`);
      return;
    }
    if (session === "unknown") {
      logEvent(`exit without session record, proceeding (${cloudGame.objectId})`);
    }

    const isHydraRunning = await isHydraLauncherRunning();

    await pendingStatusChecks.get(cloudGame.objectId)?.catch(() => {});
    await waitForBusy(cloudGame.objectId);

    invalidatePrePlayCache(cloudGame.objectId);

    const remoteNewer = useCloudSaveGuard
      .getState()
      .remoteNewerGames.includes(cloudGame.objectId);

    if (remoteNewer) {
      logEvent(`auto-sync skipped: remote newer (${cloudGame.objectId})`);
      toaster.toast({
        title: "Cloud sync skipped",
        body: `${cloudGame.title}: newer save in the cloud. Restore in Hydra, or sync to overwrite it.`,
          logo: composeToastLogo(cloudGame.iconUrl),
        });
      return;
    }

    const freshAuth = useAuthStore.getState().auth;
    const freshSubscription = useUserStore.getState().hasActiveSubscription;

    if (
      cloudGame.automaticCloudSync &&
      freshAuth &&
      freshSubscription &&
      !isHydraRunning
    ) {
      try {
        logEvent(`auto-sync start: ${cloudGame.objectId}`);
        engagePlayBlock(cloudGame.objectId);
        const result = await trackBusy(cloudGame.objectId, "sync", () =>
          syncCloudSave(
            freshAuth,
            cloudGame.objectId,
            cloudGame.shop,
            cloudGame.winePrefixPath,
            false,
            null
          )
        );

        if (result.auth) {
          useAuthStore.getState().setAuth(result.auth);
        }

        if (!result.ok && result.conflict) {
          useCloudSaveGuard.getState().flagRemoteNewer(cloudGame.objectId);
          disengagePlayBlock(cloudGame.objectId);
          toaster.toast({
            title: "Cloud save conflict",
            body: `${cloudGame.title}: changed on both sides. Choose which to keep in Hydra.`,
            logo: composeToastLogo(cloudGame.iconUrl),
          });
          return;
        }

        if (result.noop) {
          logEvent(`auto-sync no-op: unchanged (${cloudGame.objectId} v${result.version} uploaded ${result.uploadedFiles}/skipped ${result.skippedFiles})`);
          disengagePlayBlock(cloudGame.objectId);
          return;
        }

        toaster.toast({
          title: "Cloud save synced",
          body: `${cloudGame.title}: save uploaded to the cloud`,
          logo: composeToastLogo(cloudGame.iconUrl),
          icon: <PiCloudArrowUp size={20} />,
        });
        logEvent(`auto-sync done: ${cloudGame.objectId} v${result.version} uploaded ${result.uploadedFiles}/skipped ${result.skippedFiles}`);
        disengagePlayBlock(cloudGame.objectId);
      } catch (error: unknown) {
        console.error("Failed to sync cloud save", error);
        logEvent(`auto-sync failed: ${cloudGame.objectId}: ${error instanceof Error ? error.message : "unknown"}`);

        if (!useCloudSaveGuard.getState().remoteNewerGames.includes(cloudGame.objectId)) {
          disengagePlayBlock(cloudGame.objectId);
        }

        if (isRemoteNewerError(error)) {
          useCloudSaveGuard.getState().flagRemoteNewer(cloudGame.objectId);
          disengagePlayBlock(cloudGame.objectId);
          toaster.toast({
            title: "Cloud sync skipped",
            body: `${cloudGame.title}: newer save in the cloud. Restore in Hydra, or sync to overwrite it.`,
            logo: composeToastLogo(cloudGame.iconUrl),
          });
          return;
        }

        toaster.toast({
          title: "Failed to sync cloud save",
          body: error instanceof Error ? error.message : "Unknown error",
        });
      }
    } else {
      logEvent(
        `auto-sync skipped: ${cloudGame.objectId} (autoSync=${cloudGame.automaticCloudSync} auth=${Boolean(freshAuth)} sub=${Boolean(freshSubscription)} hydraRunning=${isHydraRunning})`
      );
    }
};

const onAppLifetimeNotification = (
  notification: AppLifetimeNotification
): Promise<void> => {
  const key = String(notification.unAppID);
  const seq = ++notifySeq;
  if (notification.bRunning) latestLaunchSeq = seq;
  const prior = lifetimeChains.get(key) ?? Promise.resolve();
  const next = prior.then(() => handleAppLifetimeNotification(notification, seq));
  const settled = next.catch(() => {});
  lifetimeChains.set(key, settled);
  return next.finally(() => {
    if (lifetimeChains.get(key) === settled) lifetimeChains.delete(key);
  });
};

export default definePlugin(() => {
  const { setAuth } = useAuthStore.getState();
  const { setUser } = useUserStore.getState();
  const { setLibrary } = useLibraryStore.getState();
  const { setRoute } = useNavigationStore.getState();

  registerPrePlaySync();
  registerPlayBlock();

  getAuth()
    .then((auth) => {
      if (!auth) {
        throw new Error("no auth");
      }

      setAuth(auth);

      setRoute({
        name: "home",
        params: {},
      });

      api
        .get<User>("profile/me")
        .json()
        .then((user) => {
          setUser(user);
        });

      getLibrary().then((library) => {
        setLibrary(library);
        const cleared = clearShortcutNegativeCache();
        if (cleared > 0) {
          logEvent(`shortcut cache: cleared ${cleared} negatives`);
        }
        const guard = useCloudSaveGuard.getState();
        for (const id of [...guard.remoteNewerGames]) {
          if (!library.some((game) => game.objectId === id)) {
            guard.clearRemoteNewer(id);
            logEvent(`guard prune: deleted (${id})`);
          }
        }
        const withIds = library.filter(
          (game) => game.steamShortcutAppId != null
        ).length;
        logEvent(`library shortcut ids: ${withIds}/${library.length} games`);
      });

      WSClient.connect();
    })
    .catch(() => {
      setRoute({
        name: "auth-guide",
        params: {},
      });
    });

  const { unregister: removeGameExecutionListener } =
    SteamClient.GameSessions.RegisterForAppLifetimeNotifications(
      onAppLifetimeNotification
    );

  return {
    name: "Hydra",
    titleView: <div className={staticClasses.Title}>Hydra</div>,
    content: <Plugin />,
    icon: <HydraLogo />,
    onDismount() {
      unregisterPrePlaySync();
      unregisterPlayBlock();
      removeGameExecutionListener();

      if (updateInterval) {
        clearInterval(updateInterval);
      }

      WSClient.close();
    },
  };
});
