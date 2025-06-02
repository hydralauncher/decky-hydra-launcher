import { callable } from "@decky/api";
import type { Game, Auth } from "./api-types";

export const getAuth = callable<[], Auth>("get_auth");
export const getLibrary = callable<[], Game[]>("get_library");
export const backupAndUpload = callable<
  [string, string | null, string, string],
  void
>("backup_and_upload");
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
