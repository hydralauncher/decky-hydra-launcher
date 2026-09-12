import { useCallback, useState } from "react";

import { Button, ConfirmModal, showModal } from "@decky/ui";
import { useDate } from "./hooks";
import { api } from "./hydra-api";
import { exportGameArtifact } from "./events";
import type { Game, GameArtifact } from "./api-types";
import { toaster } from "@decky/api";
import { composeToastLogo, formatBytes } from "./helpers";

export interface GameCloudSaveProps {
  artifact: GameArtifact;
  game: Game;
}

export function GameCloudSave({
  artifact,
  game,
}: GameCloudSaveProps) {
  const { formatDate, formatDateTime } = useDate();
  const [isExporting, setIsExporting] = useState(false);

  const exportArtifact = useCallback(async () => {
    setIsExporting(true);
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

      const result = await exportGameArtifact(
        response.downloadUrl,
        artifact.label ?? `Backup from ${formatDate(artifact.createdAt)}`
      );

      toaster.toast({
        title: "Backup downloaded",
        body: result.path.split("/").pop() ?? result.path,
        logo: composeToastLogo(game.iconUrl),
      });
    } catch (error: unknown) {
      console.error(error);

      toaster.toast({
        title: "Failed to download backup",
        body: "Please check if all game files are correct",
      });
    } finally {
      setIsExporting(false);
    }
  }, [artifact, formatDate, game.iconUrl]);

  const confirmArtifactDownload = useCallback(() => {
    showModal(
      <ConfirmModal
        strTitle="Confirm Backup Download"
        strDescription="Download a zip copy of this backup to your Downloads folder?"
        strOKButtonText="Download"
        strCancelButtonText="Cancel"
        onOK={exportArtifact}
      />
    );
  }, [exportArtifact]);

  return (
    <Button
      key={artifact.id}
      className="cloud-save"
      onClick={confirmArtifactDownload}
      disabled={isExporting}
    >
      <p>{artifact.label ?? `Backup from ${formatDate(artifact.createdAt)}`}</p>

      <p className="cloud-save__detail">
        {formatBytes(artifact.artifactLengthInBytes)} - {artifact.hostname}
      </p>

      <p className="cloud-save__detail">{formatDateTime(artifact.createdAt)}</p>
    </Button>
  );
}
