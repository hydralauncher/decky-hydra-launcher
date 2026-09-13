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
  invalidatePrePlayCache,
  isObjectIdCloudEligible,
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

const pendingStatusChecks = new Map<string, Promise<void>>();

const onAppLifetimeNotification = async (
  notification: AppLifetimeNotification
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

  if (updateInterval) {
    clearInterval(updateInterval);
  }

  clearGame();

  const library = await getLibrary();
  setLibrary(library);
  clearShortcutNegativeCache();

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

  logEvent(
    `app ${notification.bRunning ? "launch" : "exit"}: ${playGame.title} (${playGame.objectId})`
  );

    if (notification.bRunning) {
      const startedAt = new Date();
      lastTick = startedAt;

      setObjectId(playGame.objectId);
      setRemoteId(playGame.remoteId);
      setStartedAt(startedAt);

      if (!cloudGame) {
        logEvent(`cloud skip: ineligible (${playGame.objectId})`);
      }

      if (cloudGame) {
        disengagePlayBlock(cloudGame.objectId);

        const alreadyFlagged = useCloudSaveGuard
          .getState()
          .remoteNewerGames.includes(cloudGame.objectId);

        if (cloudGame.automaticCloudSync && auth && hasActiveSubscription && !alreadyFlagged) {
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
              if (message.includes("remote-newer")) {
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

      console.log("Started at", startedAt);

      updateInterval = setInterval(async () => {
        const secondsSinceLastTick = Math.floor(
          (new Date().getTime() - lastTick.getTime()) / 1_000
        );

        console.log("Seconds since last tick", secondsSinceLastTick);

        setElapsedTimeInMillis(Date.now() - startedAt.getTime());

        if (secondsSinceLastTick >= 10) {
          const isHydraRunning = await isHydraLauncherRunning();

          if (isHydraRunning) {
            console.log("Hydra is running, skipping playtime update");
            return;
          }

          console.log("Updating playtime", secondsSinceLastTick);
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

    if (!cloudGame) {
      logEvent(`auto-sync skipped: ineligible (${playGame.objectId})`);
      return;
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

        toaster.toast({
          title: "Cloud save synced",
          body: `${cloudGame.title}: save uploaded to the cloud`,
          logo: composeToastLogo(cloudGame.iconUrl),
          icon: <PiCloudArrowUp size={20} />,
        });
        logEvent(`auto-sync done: ${cloudGame.objectId} v${result.version}`);
        disengagePlayBlock(cloudGame.objectId);
      } catch (error: unknown) {
        console.error("Failed to sync cloud save", error);
        logEvent(`auto-sync failed: ${cloudGame.objectId}: ${error instanceof Error ? error.message : "unknown"}`);

        if (!useCloudSaveGuard.getState().remoteNewerGames.includes(cloudGame.objectId)) {
          disengagePlayBlock(cloudGame.objectId);
        }

        if (error instanceof Error && error.message.includes("remote-newer")) {
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
        clearShortcutNegativeCache();
        const guard = useCloudSaveGuard.getState();
        for (const id of [...guard.remoteNewerGames]) {
          if (!isObjectIdCloudEligible(id)) {
            guard.clearRemoteNewer(id);
            logEvent(`guard prune: ineligible (${id})`);
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
