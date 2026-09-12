import asyncio
import json
import os
import re
import tempfile

import decky

PLUGIN_DIR = decky.DECKY_PLUGIN_DIR
BACKEND_PATH = f"{PLUGIN_DIR}/bin/backend"

MAX_PLUGIN_LOGS = 10

def _plugin_log_dir() -> str | None:
    candidate = getattr(decky, "DECKY_PLUGIN_LOG_DIR", None)
    if isinstance(candidate, str) and candidate:
        return candidate
    try:
        with open(os.path.join(PLUGIN_DIR, "plugin.json"), encoding="utf-8") as handle:
            name = json.load(handle).get("name")
    except (OSError, ValueError):
        name = None
    if not isinstance(name, str) or not name:
        return None
    return os.path.join(os.path.dirname(os.path.dirname(PLUGIN_DIR)), "logs", name)

def _prune_old_logs() -> None:
    try:
        log_dir = _plugin_log_dir()
        if not log_dir:
            return
        entries = [
            os.path.join(log_dir, name)
            for name in os.listdir(log_dir)
            if name.endswith(".log")
        ]
        entries = [path for path in entries if os.path.isfile(path)]
        entries.sort(key=lambda path: os.path.getmtime(path), reverse=True)
        for stale in entries[MAX_PLUGIN_LOGS:]:
            try:
                os.remove(stale)
            except OSError:
                pass
    except OSError:
        pass

_prune_old_logs()

BACKEND_TIMEOUT = 4 * 60 * 60
STATUS_TIMEOUT = 30
SYNC_TIMEOUT = 90 * 60
RESTORE_TIMEOUT = 60 * 60

_SAFE_ARG = re.compile(r"^[A-Za-z0-9_./~ -]{1,200}$")

def _loggable_args(args: list[str]) -> str:
    return " ".join(a if _SAFE_ARG.match(a) else "<redacted>" for a in args)

async def _run_backend(args: list[str], stdin_data: str | None = None, timeout: int = BACKEND_TIMEOUT) -> str:
    decky.logger.info("backend call: %s", _loggable_args(args))

    process = await asyncio.create_subprocess_exec(
        BACKEND_PATH, *args,
        stdin=asyncio.subprocess.PIPE if stdin_data is not None else None,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    try:
        stdout, stderr = await asyncio.wait_for(
            process.communicate(stdin_data.encode() if stdin_data is not None else None),
            timeout=timeout,
        )
    except asyncio.TimeoutError:
        process.kill()
        await process.wait()
        decky.logger.error("backend timed out: %s", args[0])
        raise RuntimeError("Backend timed out")

    out = stdout.decode().strip()
    err = stderr.decode().strip()
    if process.returncode != 0:
        message = None
        try:
            payload = json.loads(out)
            if isinstance(payload, dict):
                message = payload.get("error")
        except ValueError:
            pass
        decky.logger.error("backend failed: %s: %s (stderr: %s)", args[0], message or "unknown", err)
        raise RuntimeError(message or err or "Backend failed")

    if err:
        decky.logger.info("backend stderr (%s): %s", args[0], err)
    decky.logger.info("backend ok: %s", args[0])
    return out

class Plugin:
    async def get_auth(self):
        return json.loads(await _run_backend(["get-auth"]))

    async def get_library(self):
        return json.loads(await _run_backend(["get-library"]))

    async def export_game_artifact(self, download_url: str, filename: str):
        result = await _run_backend(["export-game-artifact", download_url, filename])
        return json.loads(result)

    async def resolve_shortcut(self, app_id: str):
        result = await _run_backend(["resolve-shortcut", app_id])
        return json.loads(result)

    async def sync_cloud_save(self, auth: dict, object_id: str, shop: str, wine_prefix: str | None, force: bool, resolutions: dict | None = None):
        args = ["sync-cloud-save", object_id, wine_prefix or ""]
        args.append("force" if force else "")
        args.append(json.dumps(resolutions) if resolutions else "")
        args.append(shop)
        result = await _run_backend(args, json.dumps(auth), timeout=SYNC_TIMEOUT)
        payload = json.loads(result)
        decky.logger.info(
            "sync done for %s: version=%s files=%s uploaded=%s skipped=%s",
            object_id, payload.get("version"), payload.get("fileCount"),
            payload.get("uploadedFiles"), payload.get("skippedFiles"))
        return payload

    async def restore_cloud_save(self, auth: dict, object_id: str, shop: str, wine_prefix: str | None):
        result = await _run_backend(["restore-cloud-save", object_id, wine_prefix or "", shop], json.dumps(auth), timeout=RESTORE_TIMEOUT)
        payload = json.loads(result)
        decky.logger.info(
            "restore done for %s: version=%s restored=%s skipped=%s",
            object_id, payload.get("version"), payload.get("restoredFiles"),
            len(payload.get("skippedFiles", [])))
        return payload

    async def check_cloud_save_status(self, auth: dict, object_id: str, shop: str, wine_prefix: str | None):
        result = await _run_backend(["check-cloud-save-status", object_id, wine_prefix or "", shop], json.dumps(auth), timeout=STATUS_TIMEOUT)
        payload = json.loads(result)
        decky.logger.info(
            "status for %s: remoteNewer=%s remote=%s local=%s",
            object_id, payload.get("remoteNewer"), payload.get("remoteVersion"),
            payload.get("localVersion"))
        return payload

    async def is_hydra_launcher_running(self):
        temp_dir = tempfile.gettempdir()
        lockfile = f"{temp_dir}/hydra-launcher.lock"
        return os.path.exists(lockfile)

    async def log(self, message: str):
        decky.logger.info("frontend: %s", message)
