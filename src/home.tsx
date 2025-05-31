import { Button, PanelSection, PanelSectionRow } from "@decky/ui";
import { useCallback, useEffect } from "react";
import {
  type GameStats,
  useCurrentGame,
  useLibraryStore,
  useUserStore,
} from "./stores";
import { api } from "./hydra-api";
import { usePlaytime } from "./hooks";

export interface HomeProps {
  navigate: (name: string, params: Record<string, string>) => void;
}

export function Home({ navigate }: HomeProps) {
  const { user } = useUserStore();
  const { library } = useLibraryStore();
  const { hours, minutes, seconds } = usePlaytime();

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
  }, [objectId]);

  useEffect(() => {
    if (objectId) {
      getGameCurrentlyPlaying();
    }
  }, [objectId]);

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
        </div>
      </div>

      <PanelSection title="Playing now">
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
      </PanelSection>

      <PanelSection title="Library">
        <div className="library-games">
          {library
            .filter((game) => game.winePrefixPath)
            .map((game) => (
              <PanelSectionRow>
                <Button
                  key={game.remoteId}
                  className="library-game"
                  onClick={() =>
                    navigate("game", {
                      objectId: game.objectId,
                      winePrefixPath: game.winePrefixPath!,
                    })
                  }
                >
                  <img
                    src={game.iconUrl}
                    width="30"
                    style={{ borderRadius: 8, objectFit: "cover" }}
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
