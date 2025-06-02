import { useCallback } from "react";

import { Button, ConfirmModal, showModal } from "@decky/ui";
import { useDate } from "./hooks";
import { api } from "./hydra-api";
import { downloadGameArtifact } from "./events";
import type { Game, GameArtifact } from "./api-types";
import { toaster } from "@decky/api";
import { composeToastLogo } from "./helpers";

export interface GameCloudSaveProps {
  artifact: GameArtifact;
  game: Game;
  isGameRunning: boolean;
}

export function GameCloudSave({
  artifact,
  game,
  isGameRunning,
}: GameCloudSaveProps) {
  const { formatDate, formatDateTime } = useDate();

  const downloadArtifact = useCallback(async () => {
    toaster.toast({
      title: "Downloading backup...",
      body: "Please wait while we download the backup",
    });

    try {
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
        game.winePrefixPath!,
        response.winePrefixPath
      );

      toaster.toast({
        title: "Backup restored",
        body: "The game backup has been restored",
        logo: composeToastLogo(game.iconUrl),
      });
    } catch (error: unknown) {
      console.error(error);

      toaster.toast({
        title: "Failed to download backup",
        body: "Please check if all game files are correct",
      });
    }
  }, [artifact, game.iconUrl, game.objectId, game.winePrefixPath]);

  const confirmArtifactDownload = useCallback(() => {
    showModal(
      <ConfirmModal
        strTitle="Confirm Backup Installation"
        strDescription="Are you sure you want to install this backup? This will replace your current save."
        strOKButtonText="Install"
        strCancelButtonText="Cancel"
        onOK={downloadArtifact}
      />
    );
  }, [downloadArtifact]);

  return (
    <Button
      key={artifact.id}
      className="game-cloud-saves__cloud-save"
      onClick={confirmArtifactDownload}
      disabled={isGameRunning}
    >
      <p className="game-cloud-saves__cloud-save__title">
        {artifact.label ?? `Backup from ${formatDate(artifact.createdAt)}`}
      </p>

      <p className="game-cloud-saves__cloud-save__detail">
        {artifact.hostname}
      </p>

      <p className="game-cloud-saves__cloud-save__detail">
        {formatDateTime(artifact.createdAt)}
      </p>
    </Button>
  );
}
