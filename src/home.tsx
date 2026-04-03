import { Button, ButtonItem, PanelSection, PanelSectionRow } from "@decky/ui";
import { toaster } from "@decky/api";
import { useEffect, useMemo, useState } from "react";
import {
  useCurrentGame,
  useLibraryStore,
  useNavigationStore,
  useUserStore,
} from "./stores";
import { api } from "./hydra-api";
import { usePlaytime } from "./hooks";
import { HydraLogo, GameIcon, DownloadsSection } from "./components";
import { isHydraLauncherRunning, launchHydraBackground } from "./events";
import type { GameAssets } from "./api-types";

type LaunchState = "idle" | "searching" | "starting";

export function Home() {
  const { user, hasActiveSubscription } = useUserStore();
  const { library } = useLibraryStore();
  const { hours, minutes, seconds } = usePlaytime();
  const { setRoute } = useNavigationStore();
  const { objectId, gameAssets } = useCurrentGame();

  const [showLaunchButton, setShowLaunchButton] = useState(false);
  const [launchState, setLaunchState] = useState<LaunchState>("idle");

  const tryLaunchHydra = async (): Promise<boolean> => {
    setLaunchState("searching");
    try {
      const result = await Promise.race([
        launchHydraBackground(),
        new Promise<{ success: false; error: string }>((resolve) =>
          setTimeout(() => resolve({ success: false, error: "Search timed out after 30s" }), 30_000)
        ),
      ]);
      if (result.success) {
        setLaunchState("starting");
        setTimeout(() => setLaunchState("idle"), 3_000);
        return true;
      } else {
        setLaunchState("idle");
        return false;
      }
    } catch {
      setLaunchState("idle");
      return false;
    }
  };

  // Auto-launch on mount if Hydra is not running
  useEffect(() => {
    const init = async () => {
      try {
        const running = await isHydraLauncherRunning();
        if (running) { setShowLaunchButton(false); return; }
        const ok = await tryLaunchHydra();
        if (!ok) setShowLaunchButton(true);
      } catch {
        setShowLaunchButton(true);
      }
    };
    init();
  }, []);

  // Poll hydra running state every 3s
  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const running = await isHydraLauncherRunning();
        if (running) setShowLaunchButton(false);
      } catch {}
    }, 3_000);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    if (!objectId) return;
    api.get<GameAssets | null>(`games/steam/${objectId}`)
      .json()
      .then((assets) => { if (assets) useCurrentGame.getState().setGameAssets(assets); })
      .catch(() => {});
  }, [objectId]);

  const handleManualLaunch = async () => {
    const ok = await tryLaunchHydra();
    if (!ok) toaster.toast({ title: "Hydra", body: "Could not find Hydra executable" });
  };

  const playingNowContent = useMemo(() => {
    if (objectId) {
      return (
        <Button className="game-cover">
          <img
            src={gameAssets?.coverImageUrl}
            className="game-cover__image"
            alt={gameAssets?.title}
          />

          <div className="playtime">
            <span>
              <span className="playtime__time">{hours}</span>
              <span className="playtime__time-label">h</span>
            </span>

            <span>
              <span className="playtime__time">{minutes}</span>
              <span className="playtime__time-label">m</span>
            </span>

            <span>
              <span className="playtime__time">{seconds}</span>
              <span className="playtime__time-label">s</span>
            </span>
          </div>
        </Button>
      );
    }
    return (
      <div className="playtime-description">
        <span>No game session in progress.</span>

        <span>
          Whenever you play a game, your session playtime will show up here.
        </span>
      </div>
    );
  }, [gameAssets, objectId, hours, minutes, seconds]);

  const launchButtonLabel =
    launchState === "searching"
      ? "Searching for Hydra…"
      : launchState === "starting"
        ? "Starting Hydra…"
        : "Run downloads in background";

  return (
    <>
      <div className="user-panel">
        <Button className="user-panel__avatar">
          <img
            src={user?.profileImageUrl}
            width="64"
            height="64"
            className="user-panel__avatar-image"
            alt={user?.displayName}
          />
        </Button>

        <div style={{ display: "flex", flexDirection: "column" }}>
          <span className="user-panel__display-name">{user?.displayName}</span>
          <span className="user-panel__username">{user?.username}</span>
          {hasActiveSubscription && (
            <span className="user-panel__subscription-badge">
              <HydraLogo />
              Cloud
            </span>
          )}
        </div>
      </div>

      <PanelSection title="Playing now">{playingNowContent}</PanelSection>

      {showLaunchButton && (
        <PanelSection title="Hydra">
          <ButtonItem disabled={launchState !== "idle"} onClick={handleManualLaunch}>
            {launchButtonLabel}
          </ButtonItem>
        </PanelSection>
      )}

      <DownloadsSection />

      <PanelSection title="Playable on the Deck">
        <div className="library-games">
          {library
            .filter((game) => game.winePrefixPath)
            .map((game) => (
              <PanelSectionRow key={game.remoteId}>
                <Button
                  className="library-game"
                  onClick={() =>
                    setRoute({
                      name: "game",
                      params: {
                        game,
                      },
                    })
                  }
                >
                  <GameIcon src={game.iconUrl} alt={game.title} size={30} />
                  <span className="library-game__title">{game.title}</span>
                </Button>
              </PanelSectionRow>
            ))}
        </div>
      </PanelSection>
    </>
  );
}
