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
    
    async def backup_and_upload(self, object_id: str, wine_prefix: str, access_token: str, label: str):
        subprocess.run([BACKEND_PATH, "backup-and-upload", object_id, wine_prefix, access_token, label], capture_output=True, text=True, check=True)

    async def download_game_artifact(self, object_id: str, download_url: str, object_key: str, home_dir: str, wine_prefix: str, artifact_wine_prefix: str):
        subprocess.run([BACKEND_PATH, "download-game-artifact", object_id, download_url, object_key, home_dir, wine_prefix, artifact_wine_prefix], capture_output=True, text=True, check=True)

    async def check_if_ludusavi_binary_exists(self):
        result = subprocess.run([BACKEND_PATH, "check-if-ludusavi-binary-exists"], capture_output=True, text=True, check=True)
        return result.stdout.strip() == "true"

    async def is_hydra_launcher_running(self):
        temp_dir = tempfile.gettempdir()
        lockfile = f"{temp_dir}/hydra-launcher.lock"
        return os.path.exists(lockfile)
