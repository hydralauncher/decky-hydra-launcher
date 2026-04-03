import { callable } from "@decky/api";
import type { Game, Auth, Download } from "./api-types";

export const getAuth = callable<[], Auth>("get_auth");
export const getLibrary = callable<[], Game[]>("get_library");
export const getDownloads = callable<[], Download[]>("get_downloads");
export const backupAndUpload = callable<
  [string, string | null, string, string],
  void
>("backup_and_upload");
export const dismissDownload = callable<[string, string], { success: boolean; error?: string }>("dismiss_download");
export const launchHydraBackground = callable<
  [],
  { success: boolean; path?: string; error?: string }
>("launch_hydra_background");
export const getSteamShortcutExePaths = callable<[], string[]>("get_steam_shortcut_exe_paths");
export const updateGameSteamShortcut = callable<[string, string, number], { success: boolean; error?: string }>("update_game_steam_shortcut");
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
