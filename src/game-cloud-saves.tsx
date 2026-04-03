import { useCallback, useEffect, useState } from "react";
import { api } from "./hydra-api";
import { toaster } from "@decky/api";
import { Button, PanelSection, Spinner, ToggleField } from "@decky/ui";
import { composeToastLogo } from "./helpers";
import { useAuthStore, useCurrentGame, useUserStore } from "./stores";
import { backupAndUpload, toggleAutomaticCloudSync } from "./events";
import { CheckIcon, CloudIcon } from "./components";
import { useDate } from "./hooks";
import { GameCloudSave } from "./game-cloud-save";
import type { Game, GameArtifact } from "./api-types";

export interface GameCloudSavesProps {
  game: Game;
}

export function GameCloudSaves({ game }: GameCloudSavesProps) {
  const [isCreatingBackup, setIsCreatingBackup] = useState(false);
  const [artifacts, setArtifacts] = useState<GameArtifact[]>([]);
  const [automaticCloudSync, setAutomaticCloudSync] = useState(game.automaticCloudSync);

  const { auth } = useAuthStore();
  const { user, hasActiveSubscription } = useUserStore();
  const { objectId } = useCurrentGame();

  const { formatDate } = useDate();

  const isGameRunning = objectId === game.objectId;

  const getArtifacts = useCallback(async () => {
    const artifacts = await api
      .get<GameArtifact[]>(
        `profile/games/artifacts?objectId=${game.objectId}&shop=steam`
      )
      .json();

    setArtifacts(artifacts);
  }, [game.objectId]);

  useEffect(() => {
    getArtifacts();
  }, [getArtifacts]);

  const handleToggleCloudSync = useCallback(async (value: boolean) => {
    try {
      await toggleAutomaticCloudSync(game.shop, game.objectId, value);
      setAutomaticCloudSync(value);
    } catch (error: unknown) {
      console.error(error);
    }
  }, [game.shop, game.objectId]);

  const createNewBackup = useCallback(async () => {
    if (automaticCloudSync && auth && hasActiveSubscription) {
      setIsCreatingBackup(true);

      try {
        await backupAndUpload(
          game.objectId,
          game.winePrefixPath,
          auth.accessToken,
          `Decky Backup from ${formatDate(new Date())}`
        );

        toaster.toast({
          title: "Backup and upload successful",
          body: "The game has been backed up and uploaded to the cloud",
          logo: composeToastLogo(game.iconUrl),
        });

        getArtifacts();
      } catch (error: unknown) {
        console.error(error);

        toaster.toast({
          title: "Failed to create backup",
          body: "Please check if all game files are correct",
        });
      } finally {
        setIsCreatingBackup(false);
      }
    }
  }, [
    auth,
    automaticCloudSync,
    game.objectId,
    game.winePrefixPath,
    hasActiveSubscription,
    formatDate,
    game.iconUrl,
    getArtifacts,
  ]);

  return (
    <PanelSection title="Cloud Saves">
      <div className="game-cloud-saves__header">
        <div className="game-cloud-saves__details">
          <img
            src={game.iconUrl}
            width="30"
            style={{ borderRadius: 8, objectFit: "cover" }}
            alt={game.title}
          />

          <div>
            <span
              style={{ fontWeight: 700, color: "rgba(255, 255, 255, 0.8)" }}
            >
              {game.title}
            </span>

            {automaticCloudSync && (
              <div className="game-cloud-saves__automatic-backups">
                <CheckIcon />

                <span>Automatic backups enabled</span>
              </div>
            )}
          </div>
        </div>

        {isGameRunning && (
          <span className="game-cloud-saves__warning">
            This game is currently in session. To restore a backup, please close
            the game beforehand.
          </span>
        )}

        <span className="game-cloud-saves__info">
          Press any of the backups below to replace your current save.
        </span>
      </div>

      {hasActiveSubscription && game.executablePath && (
        <ToggleField
          label="Automatic cloud sync"
          checked={automaticCloudSync}
          onChange={handleToggleCloudSync}
        />
      )}

      <div className="game-cloud-saves__cloud-saves">
        <Button
          className="game-cloud-saves__new-backup"
          onClick={createNewBackup}
          disabled={isGameRunning}
        >
          {isCreatingBackup ? (
            <>
              <Spinner width={15} />
              Creating backup...
            </>
          ) : (
            <>
              <CloudIcon />
              New Backup
            </>
          )}
        </Button>

        {artifacts.map((artifact) => (
          <GameCloudSave
            artifact={artifact}
            game={game}
            isGameRunning={isGameRunning}
          />
        ))}
      </div>

      <span className="game-cloud-saves__used-slots">
        {artifacts.length}/{user?.quirks.backupsPerGameLimit ?? 4} save slots
        used
      </span>
    </PanelSection>
  );
}
