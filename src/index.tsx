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
  invalidatePrePlayCache,
  registerPrePlaySync,
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
        return <GameCloudSaves game={route.params.game as Game} />;
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

  const unAppID = notification.unAppID.toString();

  const game = library.find((game) => {
    return (
      game.objectId === unAppID ||
      game.winePrefixPath?.split("/").includes(unAppID)
    );
  });

  if (game) {
    logEvent(
      `app ${notification.bRunning ? "launch" : "exit"}: ${game.title} (${game.objectId})`
    );

    if (notification.bRunning) {
      const startedAt = new Date();
      lastTick = startedAt;

      setObjectId(game.objectId);
      setRemoteId(game.remoteId);
      setStartedAt(startedAt);

      disengagePlayBlock(game.objectId);

      const alreadyFlagged = useCloudSaveGuard
        .getState()
        .remoteNewerGames.includes(game.objectId);

      if (game.automaticCloudSync && auth && hasActiveSubscription && !alreadyFlagged) {
        await waitForBusy(game.objectId);

        const check = checkCloudSaveStatus(auth, game.objectId, game.shop, game.winePrefixPath)
          .then((status) => {
            if (status.auth) {
              useAuthStore.getState().setAuth(status.auth);
            }
            if (status.remoteNewer) {
              useCloudSaveGuard.getState().flagRemoteNewer(game.objectId);
              logEvent(`guard flagged: ${game.objectId} (remote v${status.remoteVersion}, local v${status.localVersion ?? "none"})`);
              if (useCurrentGame.getState().objectId === game.objectId) {
                toaster.toast({
                  title: "Newer cloud save available",
                  body: `${game.title} has a newer save in the cloud. This session will not sync — restore it from the Hydra plugin to keep the cloud version.`,
                  logo: composeToastLogo(game.iconUrl),
                });
              }
            } else {
              useCloudSaveGuard.getState().clearRemoteNewer(game.objectId);
              logEvent(`guard clear: ${game.objectId} (remote v${status.remoteVersion})`);
            }
          })
          .catch((err) => {
            console.error("Failed to check cloud save status", err);
            useCloudSaveGuard.getState().flagRemoteNewer(game.objectId);
            toaster.toast({
              title: "Cloud save status unknown",
              body: `Could not check the cloud save for ${game.title}. This session will not auto-sync as a precaution.`,
            });
          })
          .finally(() => {
            if (pendingStatusChecks.get(game.objectId) === check) {
              pendingStatusChecks.delete(game.objectId);
            }
          });

        pendingStatusChecks.set(game.objectId, check);
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
            .put(`profile/games/${game.remoteId}`, {
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

    const isHydraRunning = await isHydraLauncherRunning();

    await pendingStatusChecks.get(game.objectId)?.catch(() => {});
    await waitForBusy(game.objectId);

    invalidatePrePlayCache(game.objectId);

    const remoteNewer = useCloudSaveGuard
      .getState()
      .remoteNewerGames.includes(game.objectId);

    if (remoteNewer) {
      toaster.toast({
        title: "Cloud sync skipped",
        body: `${game.title} has a newer save in the cloud. Restore it from the Hydra plugin, or sync manually to overwrite the cloud version.`,
        logo: composeToastLogo(game.iconUrl),
      });
      return;
    }

    const freshAuth = useAuthStore.getState().auth;
    const freshSubscription = useUserStore.getState().hasActiveSubscription;

    if (
      game.automaticCloudSync &&
      freshAuth &&
      freshSubscription &&
      !isHydraRunning
    ) {
      try {
        logEvent(`auto-sync start: ${game.objectId}`);
        engagePlayBlock(game.objectId);
        const result = await trackBusy(game.objectId, "sync", () =>
          syncCloudSave(
            freshAuth,
            game.objectId,
            game.shop,
            game.winePrefixPath,
            false,
            null
          )
        );

        if (result.auth) {
          useAuthStore.getState().setAuth(result.auth);
        }

        if (!result.ok && result.conflict) {
          useCloudSaveGuard.getState().flagRemoteNewer(game.objectId);
          disengagePlayBlock(game.objectId);
          toaster.toast({
            title: "Cloud save conflict",
            body: `${game.title}: ${result.conflict.length} file(s) changed on both this device and the cloud. Open the Hydra plugin to choose which to keep.`,
            logo: composeToastLogo(game.iconUrl),
          });
          return;
        }

        toaster.toast({
          title: "Cloud save synced",
          body: `${game.title} save has been uploaded to the cloud`,
          logo: composeToastLogo(game.iconUrl),
          icon: <PiCloudArrowUp size={20} />,
        });
        logEvent(`auto-sync done: ${game.objectId} v${result.version}`);
        disengagePlayBlock(game.objectId);
      } catch (error: unknown) {
        console.error("Failed to sync cloud save", error);
        logEvent(`auto-sync failed: ${game.objectId}: ${error instanceof Error ? error.message : "unknown"}`);

        if (!useCloudSaveGuard.getState().remoteNewerGames.includes(game.objectId)) {
          disengagePlayBlock(game.objectId);
        }

        if (error instanceof Error && error.message.includes("remote-newer")) {
          useCloudSaveGuard.getState().flagRemoteNewer(game.objectId);
          disengagePlayBlock(game.objectId);
          toaster.toast({
            title: "Cloud sync skipped",
            body: `${game.title} has a newer save in the cloud. Restore it from the Hydra plugin, or sync manually to overwrite the cloud version.`,
            logo: composeToastLogo(game.iconUrl),
          });
          return;
        }

        toaster.toast({
          title: "Failed to sync cloud save",
          body: error instanceof Error ? error.message : "Unknown error",
        });
      }
    }
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
