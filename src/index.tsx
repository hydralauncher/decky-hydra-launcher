import { staticClasses } from "@decky/ui";
import { definePlugin, toaster } from "@decky/api";
import { useEffect, useMemo } from "react";
import { AppLifetimeNotification } from "@decky/ui/dist/globals/steam-client/GameSessions";
import styles from "./styles/globals.scss";
import {
  type User,
  useAuthStore,
  useCurrentGame,
  useLibraryStore,
  useNavigationStore,
  useUserStore,
} from "./stores";
import { api } from "./hydra-api";
import { Home } from "./home";
import { WSClient } from "./ws";
import { composeToastLogo } from "./helpers";
import { GameCloudSaves } from "./game-cloud-saves";
import { AuthGuide } from "./auth-guide";
import { backupAndUpload, getLibrary, isHydraLauncherRunning } from "./events";
import { getAuth } from "./events";
import { HydraLogo } from "./components";
import { formatDate } from "./hooks";

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
  }, [auth]);

  const content = useMemo(() => {
    switch (route?.name) {
      case "auth-guide":
        return <AuthGuide />;
      case "game":
        return <GameCloudSaves game={route.params.game} />;
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
    return game.objectId === unAppID || game.winePrefixPath?.includes(unAppID);
  });

  if (game) {
    if (notification.bRunning) {
      const startedAt = new Date();
      lastTick = startedAt;

      setObjectId(game.objectId);
      setRemoteId(game.remoteId);
      setStartedAt(startedAt);

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

    // Check if there's any chance for the accessToken to be expired
    if (
      game.automaticCloudSync &&
      auth &&
      hasActiveSubscription &&
      !isHydraRunning
    ) {
      await backupAndUpload(
        game.objectId,
        game.winePrefixPath,
        auth.accessToken,
        `Automatic Decky Backup from ${formatDate(new Date(), "en")}`
      );

      toaster.toast({
        title: "Backup and upload successful",
        body: "The game has been backed up and uploaded to the cloud",
        logo: composeToastLogo(game.iconUrl),
      });
    }
  }
};

export default definePlugin(() => {
  const { setAuth } = useAuthStore.getState();
  const { setUser } = useUserStore.getState();
  const { setLibrary } = useLibraryStore.getState();
  const { setRoute } = useNavigationStore.getState();

  getAuth()
    .then((auth) => {
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

      getLibrary().then((library) => setLibrary(library));

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
      removeGameExecutionListener();

      if (updateInterval) {
        clearInterval(updateInterval);
      }

      WSClient.close();
    },
  };
});
