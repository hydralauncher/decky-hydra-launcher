import asyncio
import json
import os
import tempfile

import decky

PLUGIN_DIR = decky.DECKY_PLUGIN_DIR
BACKEND_PATH = f"{PLUGIN_DIR}/bin/backend"

# Sync/restore can involve large transfers; status checks must stay quick so
# the exit handler never waits long on them.
BACKEND_TIMEOUT = 4 * 60 * 60
STATUS_TIMEOUT = 30


async def _run_backend(args: list[str], stdin_data: str | None = None, timeout: int = BACKEND_TIMEOUT) -> str:
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
        raise RuntimeError("Backend timed out")

    out = stdout.decode().strip()
    if process.returncode != 0:
        message = None
        try:
            payload = json.loads(out)
            if isinstance(payload, dict):
                message = payload.get("error")
        except ValueError:
            pass
        raise RuntimeError(message or stderr.decode().strip() or "Backend failed")
    return out


class Plugin:
    async def get_auth(self):
        return json.loads(await _run_backend(["get-auth"]))

    async def get_library(self):
        return json.loads(await _run_backend(["get-library"]))

    async def download_game_artifact(self, object_id: str, download_url: str, object_key: str, home_dir: str, wine_prefix: str, artifact_wine_prefix: str | None):
        await _run_backend(["download-game-artifact", object_id, download_url, object_key, home_dir, wine_prefix, artifact_wine_prefix or ""])

    async def check_if_ludusavi_binary_exists(self):
        result = await _run_backend(["check-if-ludusavi-binary-exists"])
        return result == "true"

    async def sync_cloud_save(self, auth: dict, object_id: str, wine_prefix: str | None, force: bool):
        # Auth goes through stdin so tokens never appear in the process list.
        args = ["sync-cloud-save", object_id, wine_prefix or ""]
        if force:
            args.append("force")
        result = await _run_backend(args, json.dumps(auth))
        return json.loads(result)

    async def restore_cloud_save(self, auth: dict, object_id: str, wine_prefix: str | None):
        result = await _run_backend(["restore-cloud-save", object_id, wine_prefix or ""], json.dumps(auth))
        return json.loads(result)

    async def check_cloud_save_status(self, auth: dict, object_id: str, wine_prefix: str | None):
        result = await _run_backend(["check-cloud-save-status", object_id, wine_prefix or ""], json.dumps(auth), timeout=STATUS_TIMEOUT)
        return json.loads(result)

    async def is_hydra_launcher_running(self):
        temp_dir = tempfile.gettempdir()
        lockfile = f"{temp_dir}/hydra-launcher.lock"
        return os.path.exists(lockfile)
