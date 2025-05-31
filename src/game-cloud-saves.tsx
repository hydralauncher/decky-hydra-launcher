import { useCallback, useEffect, useState } from "react";
import { api } from "./hydra-api";
import { toaster } from "@decky/api";
import { Button, PanelSection } from "@decky/ui";
import { composeToastLogo } from "./helpers";
import { useAuthStore, useUserStore, type Game } from "./stores";
import { backupAndUpload, downloadGameArtifact } from "./events";
import { CloudIcon } from "./components";
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

  const { formatDate } = useDate();

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
      await backupAndUpload(
        game.objectId,
        game.winePrefixPath,
        auth.accessToken
      );

      toaster.toast({
        title: "Backup and upload successful",
        body: "The game has been backed up and uploaded to the cloud",
        logo: composeToastLogo(game.iconUrl),
      });

      getArtifacts();
    }
  }, [
    auth,
    game.automaticCloudSync,
    game.objectId,
    game.winePrefixPath,
    hasActiveSubscription,
  ]);

  return (
    <PanelSection title="Cloud Saves">
      <div className="game-cloud-saves__header">
        <img
          src={game.iconUrl}
          width="30"
          style={{ borderRadius: 8, objectFit: "cover" }}
          alt={game.title}
        />

        <span>{game.title}</span>
      </div>

      {game.automaticCloudSync && <span>Automatic backups enabled</span>}

      <span>
        This game is currently in session. To restore a backup, please close the
        game beforehand.
      </span>

      <div className="game-cloud-saves__cloud-saves">
        <Button
          className="game-cloud-saves__new-backup"
          onClick={createNewBackup}
        >
          <CloudIcon />
          New Backup
        </Button>

        {artifacts.map((artifact) => (
          <Button
            key={artifact.id}
            className="game-cloud-saves__cloud-save"
            onClick={() => downloadArtifact(artifact)}
          >
            <span>
              {artifact.label ??
                `Backup from ${formatDate(artifact.createdAt)}`}
            </span>

            <span>{}</span>
          </Button>
        ))}
      </div>

      <span>
        {artifacts.length}/{user?.quirks.backupsPerGameLimit ?? 4} save slots
        used
      </span>
    </PanelSection>
  );
}
