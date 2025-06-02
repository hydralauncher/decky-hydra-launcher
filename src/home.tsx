import { Button, PanelSection, PanelSectionRow } from "@decky/ui";
import { useCallback, useEffect, useMemo } from "react";
import {
  useCurrentGame,
  useLibraryStore,
  useNavigationStore,
  useUserStore,
} from "./stores";
import { api } from "./hydra-api";
import { usePlaytime } from "./hooks";
import { HydraLogo } from "./components/hydra-logo";
import type { GameStats } from "./api-types";

export function Home() {
  const { user, hasActiveSubscription } = useUserStore();
  const { library } = useLibraryStore();
  const { hours, minutes, seconds } = usePlaytime();

  const { setRoute } = useNavigationStore();

  const { objectId, gameStats, setGameStats } = useCurrentGame();

  const getGameCurrentlyPlaying = useCallback(async () => {
    const searchParams = new URLSearchParams({
      objectId: objectId!,
      shop: "steam",
    });

    const stats = await api
      .get<GameStats>(`games/stats?${searchParams.toString()}`)
      .json();

    setGameStats(stats);
  }, [objectId, setGameStats]);

  useEffect(() => {
    if (objectId) {
      getGameCurrentlyPlaying();
    }
  }, [objectId, getGameCurrentlyPlaying]);

  const playingNowContent = useMemo(() => {
    if (objectId) {
      return (
        <Button className="game-cover">
          <img
            src={gameStats?.assets.coverImageUrl}
            className="game-cover__image"
            alt={gameStats?.assets.title}
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
  }, [gameStats, objectId, hours, minutes, seconds]);

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

      <PanelSection title="Playable on the Deck">
        <div className="library-games">
          {library
            .filter((game) => game.winePrefixPath)
            .map((game) => (
              <PanelSectionRow>
                <Button
                  key={game.remoteId}
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
                  <img
                    src={game.iconUrl}
                    width="30"
                    style={{ borderRadius: 8, objectFit: "cover" }}
                    alt={game.title}
                  />
                  <span className="library-game__title">{game.title}</span>
                </Button>
              </PanelSectionRow>
            ))}
        </div>
      </PanelSection>
    </>
  );
}
