import { useCallback, useEffect, useState } from "react";
import { api } from "./hydra-api";
import { toaster } from "@decky/api";
import { Button, ConfirmModal, PanelSection, Spinner, showModal } from "@decky/ui";
import { composeToastLogo, formatBytes } from "./helpers";
import { useAuthStore, useCurrentGame, useUserStore } from "./stores";
import { restoreCloudSave, syncCloudSave } from "./events";
import { CheckIcon, CloudIcon } from "./components";
import { useDate } from "./hooks";
import { GameCloudSave } from "./game-cloud-save";
import type { CloudSaveSnapshotSummary, Game, GameArtifact } from "./api-types";

export interface GameCloudSavesProps {
  game: Game;
}

export function GameCloudSaves({ game }: GameCloudSavesProps) {
  const [isSyncing, setIsSyncing] = useState(false);
  const [isRestoring, setIsRestoring] = useState(false);
  const [snapshot, setSnapshot] = useState<CloudSaveSnapshotSummary | null>(
    null
  );
  const [artifacts, setArtifacts] = useState<GameArtifact[]>([]);

  const { auth, setAuth } = useAuthStore();
  const { hasActiveSubscription } = useUserStore();
  const { objectId } = useCurrentGame();

  const { formatDateTime } = useDate();

  const isGameRunning = objectId === game.objectId;
  const canSync = Boolean(auth && hasActiveSubscription);

  const getSnapshot = useCallback(async () => {
    try {
      const snapshots = await api
        .get<CloudSaveSnapshotSummary[]>(
          `profile/cloud-saves/snapshots?objectId=${game.objectId}&shop=steam`
        )
        .json();

      const latest = snapshots.sort((a, b) => b.version - a.version)[0];
      setSnapshot(latest ?? null);
    } catch (error: unknown) {
      console.error("Failed to load cloud save snapshot", error);
      setSnapshot(null);
    }
  }, [game.objectId]);

  const getLegacyArtifacts = useCallback(async () => {
    try {
      const artifacts = await api
        .get<GameArtifact[]>(
          `profile/games/artifacts?objectId=${game.objectId}&shop=steam`
        )
        .json();

      setArtifacts(artifacts);
    } catch (error: unknown) {
      console.error("Failed to load legacy backups", error);
      setArtifacts([]);
    }
  }, [game.objectId]);

  useEffect(() => {
    getSnapshot();
    getLegacyArtifacts();
  }, [getSnapshot, getLegacyArtifacts]);

  const syncNow = useCallback(async () => {
    if (!auth || !hasActiveSubscription) return;

    setIsSyncing(true);

    try {
      const result = await syncCloudSave(
        auth,
        game.objectId,
        game.winePrefixPath
      );

      if (result.auth) setAuth(result.auth);

      toaster.toast({
        title: "Cloud save synced",
        body: `Uploaded ${result.uploadedFiles} files (${result.skippedFiles} already in the cloud)`,
        logo: composeToastLogo(game.iconUrl),
      });

      getSnapshot();
    } catch (error: unknown) {
      console.error(error);

      toaster.toast({
        title: "Failed to sync cloud save",
        body: error instanceof Error ? error.message : "Unknown error",
      });
    } finally {
      setIsSyncing(false);
    }
  }, [
    auth,
    hasActiveSubscription,
    game.objectId,
    game.winePrefixPath,
    game.iconUrl,
    setAuth,
    getSnapshot,
  ]);

  const restore = useCallback(async () => {
    if (!auth || !hasActiveSubscription) return;

    setIsRestoring(true);

    toaster.toast({
      title: "Restoring cloud save...",
      body: "Please wait while we download and install your save",
    });

    try {
      const result = await restoreCloudSave(
        auth,
        game.objectId,
        game.winePrefixPath
      );

      if (result.auth) setAuth(result.auth);

      const skippedNote = result.skippedFiles.length
        ? ` (${result.skippedFiles.length} files skipped)`
        : "";

      toaster.toast({
        title: "Cloud save restored",
        body: `Restored ${result.restoredFiles} files${skippedNote}`,
        logo: composeToastLogo(game.iconUrl),
      });
    } catch (error: unknown) {
      console.error(error);

      toaster.toast({
        title: "Failed to restore cloud save",
        body: error instanceof Error ? error.message : "Unknown error",
      });
    } finally {
      setIsRestoring(false);
    }
  }, [
    auth,
    hasActiveSubscription,
    game.objectId,
    game.winePrefixPath,
    game.iconUrl,
    setAuth,
  ]);

  const confirmRestore = useCallback(() => {
    showModal(
      <ConfirmModal
        strTitle="Confirm Cloud Save Restore"
        strDescription="Are you sure you want to restore this cloud save? This will replace your current local save files."
        strOKButtonText="Restore"
        strCancelButtonText="Cancel"
        onOK={restore}
      />
    );
  }, [restore]);

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

            {game.automaticCloudSync && (
              <div className="game-cloud-saves__automatic-backups">
                <CheckIcon />

                <span>Automatic backups enabled</span>
              </div>
            )}
          </div>
        </div>

        {isGameRunning && (
          <span className="game-cloud-saves__warning">
            This game is currently in session. To sync or restore a cloud save,
            please close the game beforehand.
          </span>
        )}

        <span className="game-cloud-saves__info">
          {snapshot
            ? `Version ${snapshot.version} - ${snapshot.fileCount} files - ${formatBytes(snapshot.totalSizeBytes)} - ${formatDateTime(snapshot.updatedAt)}`
            : "No cloud save snapshot found for this game yet."}
        </span>
      </div>

      <div className="game-cloud-saves__cloud-saves">
        <Button
          className="game-cloud-saves__new-backup"
          onClick={syncNow}
          disabled={isGameRunning || !canSync || isSyncing || isRestoring}
        >
          {isSyncing ? (
            <>
              <Spinner width={15} />
              Syncing...
            </>
          ) : (
            <>
              <CloudIcon />
              Sync Now
            </>
          )}
        </Button>

        <Button
          className="cloud-save"
          onClick={confirmRestore}
          disabled={
            isGameRunning || !canSync || !snapshot || isSyncing || isRestoring
          }
        >
          {isRestoring ? (
            <>
              <Spinner width={15} />
              Restoring...
            </>
          ) : (
            "Restore Cloud Save"
          )}
        </Button>
      </div>

      {artifacts.length > 0 && (
        <PanelSection title="Legacy Backups (read-only)">
          <span className="game-cloud-saves__info">
            Backups created with the old system. They can only be restored, not
            replaced.
          </span>

          {artifacts.map((artifact) => (
            <GameCloudSave
              key={artifact.id}
              artifact={artifact}
              game={game}
              isGameRunning={isGameRunning}
            />
          ))}
        </PanelSection>
      )}
    </PanelSection>
  );
}
