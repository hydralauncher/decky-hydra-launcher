import { callable } from "@decky/api";
import { type Game, type Auth } from "./stores";

export const getAuth = callable<[], Auth>("get_auth");
export const getLibrary = callable<[], Game[]>("get_library");
export const backupAndUpload = callable<[string, string | null, string], void>(
  "backup_and_upload"
);
export const isHydraLauncherRunning = callable<[], boolean>(
  "is_hydra_launcher_running"
);
export const downloadGameArtifact = callable<
  [string, string, string, string, string | null, string],
  void
>("download_game_artifact");
