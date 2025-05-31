import { useCallback, useEffect, useState } from "react";
import { api } from "./hydra-api";
import { toaster } from "@decky/api";
import { Button, PanelSection } from "@decky/ui";
import { composeToastLogo } from "./helpers";
import {
  useAuthStore,
  useCurrentGame,
  useUserStore,
  type Game,
} from "./stores";
import { backupAndUpload, downloadGameArtifact } from "./events";
import { CheckIcon, CloudIcon } from "./components";
import { useDate } from "./hooks";

interface GameArtifact {
  id: string;
  artifactLengthInBytes: number;
  downloadOptionTitle: string | null;
  createdAt: string;
  updatedAt: string;
  hostname: string;
  downloadCount: number;
  label?: string;
}

export interface GameCloudSavesProps {
  game: Game;
}

export function GameCloudSaves({ game }: GameCloudSavesProps) {
  const [artifacts, setArtifacts] = useState<GameArtifact[]>([]);
  const { auth } = useAuthStore();
  const { user, hasActiveSubscription } = useUserStore();
  const { objectId } = useCurrentGame();

  const { formatDate, formatDateTime } = useDate();

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

  const downloadArtifact = useCallback(async (artifact: GameArtifact) => {
    const response = await api
      .post<{
        downloadUrl: string;
        objectKey: string;
        homeDir: string;
        winePrefixPath: string | null;
      }>(`profile/games/artifacts/${artifact.id}/download`)
      .json();

    await downloadGameArtifact(
      game.objectId,
      response.downloadUrl,
      response.objectKey,
      response.homeDir,
      response.winePrefixPath,
      game.winePrefixPath!
    );

    toaster.toast({
      title: "Backup restored",
      body: "The game backup has been restored",
      logo: composeToastLogo(game.iconUrl),
    });
  }, []);

  const createNewBackup = useCallback(async () => {
    if (game.automaticCloudSync && auth && hasActiveSubscription) {
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
      } catch (error) {
        toaster.toast({
          title: "Failed to create backup",
          body: "Please check if all game files are correct",
        });
      }
    }
  }, [
    auth,
    game.automaticCloudSync,
    game.objectId,
    game.winePrefixPath,
    hasActiveSubscription,
    formatDate,
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

          <span style={{ fontWeight: 700 }}>{game.title}</span>
        </div>

        {isGameRunning && (
          <span className="game-cloud-saves__warning">
            This game is currently in session. To restore a backup, please close
            the game beforehand.
          </span>
        )}

        {game.automaticCloudSync && (
          <div className="game-cloud-saves__automatic-backups">
            <CheckIcon />

            <span>Automatic backups enabled</span>
          </div>
        )}
      </div>

      <div className="game-cloud-saves__cloud-saves">
        <Button
          className="game-cloud-saves__new-backup"
          onClick={createNewBackup}
          disabled={isGameRunning}
        >
          <CloudIcon />
          New Backup
        </Button>

        {artifacts.map((artifact) => (
          <Button
            key={artifact.id}
            className="game-cloud-saves__cloud-save"
            onClick={() => downloadArtifact(artifact)}
            disabled={isGameRunning}
          >
            <span className="game-cloud-saves__cloud-save__title">
              {artifact.label ??
                `Backup from ${formatDate(artifact.createdAt)}`}
            </span>

            <span className="game-cloud-saves__cloud-save__date">
              {formatDateTime(artifact.createdAt)}
            </span>
          </Button>
        ))}
      </div>

      <span className="game-cloud-saves__info">
        {artifacts.length}/{user?.quirks.backupsPerGameLimit ?? 4} save slots
        used
      </span>
    </PanelSection>
  );
}
