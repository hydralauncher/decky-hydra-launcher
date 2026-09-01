import { callable } from "@decky/api";
import type {
  Auth,
  CloudSaveRestoreResult,
  CloudSaveStatus,
  CloudSaveSyncResult,
  Game,
} from "./api-types";

export const getAuth = callable<[], Auth>("get_auth");
export const getLibrary = callable<[], Game[]>("get_library");
export const isHydraLauncherRunning = callable<[], boolean>(
  "is_hydra_launcher_running"
);
export const downloadGameArtifact = callable<
  [string, string, string, string, string, string | null],
  void
>("download_game_artifact");
export const checkIfLudusaviBinaryExists = callable<[], boolean>(
  "check_if_ludusavi_binary_exists"
);
export const syncCloudSave = callable<
  [Auth, string, string | null, boolean],
  CloudSaveSyncResult
>("sync_cloud_save");
export const restoreCloudSave = callable<
  [Auth, string, string | null],
  CloudSaveRestoreResult
>("restore_cloud_save");
export const checkCloudSaveStatus = callable<
  [Auth, string, string | null],
  CloudSaveStatus
>("check_cloud_save_status");
