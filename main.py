import os
import subprocess
import json
import tempfile
import decky

PLUGIN_DIR = decky.DECKY_PLUGIN_DIR
BACKEND_PATH = f"{PLUGIN_DIR}/bin/backend"

class Plugin:
    async def get_auth(self):
        result = subprocess.run([BACKEND_PATH, "get-auth"], capture_output=True, text=True, check=True)
        return json.loads(result.stdout)

    async def get_library(self):
        result = subprocess.run([BACKEND_PATH, "get-library"], capture_output=True, text=True, check=True)
        return json.loads(result.stdout)

    async def get_downloads(self):
        try:
            result = subprocess.run(
                [BACKEND_PATH, "get-downloads"],
                capture_output=True, text=True, check=True, timeout=5
            )
            return json.loads(result.stdout)
        except subprocess.TimeoutExpired:
            decky.logger.error("get_downloads timed out")
            return []
        except Exception as e:
            decky.logger.error(f"get_downloads failed: {e}")
            return []

    async def backup_and_upload(self, object_id: str, wine_prefix: str, access_token: str, label: str):
        subprocess.run([BACKEND_PATH, "backup-and-upload", object_id, wine_prefix, access_token, label], capture_output=True, text=True, check=True)

    async def download_game_artifact(self, object_id: str, download_url: str, object_key: str, home_dir: str, wine_prefix: str, artifact_wine_prefix: str | None):
        subprocess.run([BACKEND_PATH, "download-game-artifact", object_id, download_url, object_key, home_dir, wine_prefix, artifact_wine_prefix or ""], capture_output=True, text=True, check=True)

    async def check_if_ludusavi_binary_exists(self):
        result = subprocess.run([BACKEND_PATH, "check-if-ludusavi-binary-exists"], capture_output=True, text=True, check=True)
        return result.stdout.strip() == "true"

    async def launch_hydra_background(self):
        home = os.path.expanduser("~")
        decky.logger.info(f"[Hydra] Searching for executable, home={home}")

        try:
            # Step 1 — fast: known fixed paths (no filesystem scan)
            fast_candidates = [
                f"{home}/AppImages/hydra.AppImage",
                f"{home}/Applications/hydra.AppImage",
                f"{home}/.local/bin/hydra",
                "/usr/bin/hydra",
                "/usr/local/bin/hydra",
                "/opt/hydra/hydra",
            ]
            for path in fast_candidates:
                if os.path.isfile(path) and os.access(path, os.X_OK):
                    decky.logger.info(f"[Hydra] Found (fast): {path}")
                    return self._start_hydra(path)

            # Step 2 — shallow: case-insensitive find in common dirs, 5s timeout each
            common_dirs = [
                f"{home}/AppImages",
                f"{home}/Applications",
                f"{home}/.local/bin",
                f"{home}/Downloads",
                home,
                "/opt",
            ]
            for directory in common_dirs:
                if not os.path.isdir(directory):
                    continue
                try:
                    result = subprocess.run(
                        ["find", directory, "-maxdepth", "2", "-iname", "hydra.appimage",
                         "-o", "-maxdepth", "2", "-iname", "hydra"],
                        capture_output=True, text=True, timeout=5
                    )
                    for line in result.stdout.strip().splitlines():
                        path = line.strip()
                        if path and os.path.isfile(path) and os.access(path, os.X_OK):
                            decky.logger.info(f"[Hydra] Found (shallow): {path}")
                            return self._start_hydra(path)
                except subprocess.TimeoutExpired:
                    decky.logger.warning(f"[Hydra] Shallow search timed out in {directory}")

            # Step 3 — deep: recursive, hard cap at 20s
            decky.logger.info("[Hydra] Starting deep search (20s timeout)…")
            try:
                result = subprocess.run(
                    ["find", home, "-iname", "hydra.appimage", "-o", "-iname", "hydra"],
                    capture_output=True, text=True, timeout=20
                )
                for line in result.stdout.strip().splitlines():
                    path = line.strip()
                    if path and os.path.isfile(path) and os.access(path, os.X_OK):
                        decky.logger.info(f"[Hydra] Found (deep): {path}")
                        return self._start_hydra(path)
            except subprocess.TimeoutExpired:
                decky.logger.warning("[Hydra] Deep search timed out after 20s")

            decky.logger.error("[Hydra] Executable not found anywhere")
            return {"success": False, "error": "Hydra not found. Make sure it is installed."}

        except Exception as e:
            decky.logger.error(f"[Hydra] launch_hydra_background unexpected error: {e}")
            return {"success": False, "error": str(e)}

    def _start_hydra(self, executable: str):
        import time

        log_path = "/tmp/hydra-decky-launch.log"
        try:
            env = os.environ.copy()

            # Strip Steam/Decky library paths — they break /bin/bash inside AppImages
            # (causes "undefined symbol: rl_trim_arg_from_keyseq")
            for var in ("LD_LIBRARY_PATH", "LD_PRELOAD"):
                old = env.pop(var, None)
                if old:
                    decky.logger.info(f"[Hydra] Stripped {var}={old}")

            # Ensure a display is set — Gamescope uses Wayland
            if not env.get("DISPLAY") and not env.get("WAYLAND_DISPLAY"):
                env["DISPLAY"] = ":0"
                decky.logger.warning("[Hydra] No display env found, forcing DISPLAY=:0")

            decky.logger.info(f"[Hydra] DISPLAY={env.get('DISPLAY')} WAYLAND_DISPLAY={env.get('WAYLAND_DISPLAY')}")

            with open(log_path, "w") as log_file:
                proc = subprocess.Popen(
                    [executable, "--hidden"],
                    stdout=log_file,
                    stderr=log_file,
                    start_new_session=True,
                    env=env,
                )

            decky.logger.info(f"[Hydra] Process started pid={proc.pid}, log at {log_path}")

            # Wait up to 5s for the lockfile to confirm Hydra actually started
            lockfile = os.path.join(tempfile.gettempdir(), "hydra-launcher.lock")
            for _ in range(10):
                time.sleep(0.5)
                if os.path.exists(lockfile):
                    decky.logger.info("[Hydra] Lockfile confirmed — Hydra is running")
                    return {"success": True, "path": executable}

            # Check if process already died
            ret = proc.poll()
            if ret is not None:
                decky.logger.error(f"[Hydra] Process exited immediately with code {ret}, see {log_path}")
                try:
                    with open(log_path) as f:
                        output = f.read(2000)
                    decky.logger.error(f"[Hydra] Output: {output}")
                except Exception:
                    pass
                return {"success": False, "error": f"Hydra exited immediately (code {ret}). Check /tmp/hydra-decky-launch.log"}

            decky.logger.warning("[Hydra] Lockfile not found after 5s but process is still running")
            return {"success": True, "path": executable}

        except Exception as e:
            decky.logger.error(f"[Hydra] Failed to start {executable}: {e}")
            return {"success": False, "error": f"Failed to start: {e}"}

    async def update_game_steam_shortcut(self, shop: str, object_id: str, app_id: int):
        try:
            args = [BACKEND_PATH, "update-game-steam-shortcut", shop, object_id, str(app_id)]
            result = subprocess.run(args, capture_output=True, text=True, timeout=5)
            if result.returncode == 0:
                return {"success": True}
            error = result.stderr.strip()
            decky.logger.error(f"[Hydra] update-game-steam-shortcut failed: {error}")
            return {"success": False, "error": error}
        except Exception as e:
            decky.logger.error(f"[Hydra] update_game_steam_shortcut error: {e}")
            return {"success": False, "error": str(e)}

    async def dismiss_download(self, shop: str, object_id: str):
        try:
            result = subprocess.run(
                [BACKEND_PATH, "delete-download", shop, object_id],
                capture_output=True, text=True, timeout=5
            )
            if result.returncode == 0:
                decky.logger.info(f"[Hydra] Deleted download {shop}:{object_id}")
                return {"success": True}
            else:
                error = result.stderr.strip()
                decky.logger.error(f"[Hydra] delete-download failed: {error}")
                return {"success": False, "error": error}
        except Exception as e:
            decky.logger.error(f"[Hydra] dismiss_download error: {e}")
            return {"success": False, "error": str(e)}

    async def get_steam_shortcut_exe_paths(self) -> list:
        """Read shortcuts.vdf for all Steam users and return all exe paths."""
        from pathlib import Path
        exe_paths = []
        userdata = Path(decky.DECKY_USER_HOME) / ".local" / "share" / "Steam" / "userdata"
        if not userdata.is_dir():
            return exe_paths
        for user_dir in userdata.iterdir():
            if not user_dir.is_dir() or not user_dir.name.isdigit():
                continue
            shortcuts_path = user_dir / "config" / "shortcuts.vdf"
            if not shortcuts_path.is_file():
                continue
            try:
                data = shortcuts_path.read_bytes()
                i = 0
                while i < len(data):
                    if data[i] == 0x01:
                        j = i + 1
                        while j < len(data) and data[j] != 0:
                            j += 1
                        key = data[i + 1:j].decode("utf-8", errors="ignore").lower()
                        start = j + 1
                        end = data.find(b"\x00", start)
                        if end == -1:
                            break
                        if key == "exe":
                            path = data[start:end].decode("utf-8", errors="ignore").strip('"')
                            if path:
                                exe_paths.append(path)
                                decky.logger.info(f"[Hydra] shortcut exe: {path}")
                        i = end + 1
                    else:
                        i += 1
            except Exception as e:
                decky.logger.error(f"[Hydra] Failed to read shortcuts.vdf: {e}")
        return exe_paths

    async def is_hydra_launcher_running(self):
        temp_dir = tempfile.gettempdir()
        lockfile = f"{temp_dir}/hydra-launcher.lock"
        return os.path.exists(lockfile)
