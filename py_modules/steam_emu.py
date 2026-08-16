import os
from pathlib import Path
import decky

USERNAME_KEY = "UserName="
LANGUAGE_KEY = "Language="


def find_ini(executable_path: str):
    if not executable_path:
        return None
    directory = os.path.dirname(executable_path)
    if not os.path.isdir(directory):
        return None
    try:
        for entry in os.listdir(directory):
            if entry.lower() == "steam_emu.ini":
                return os.path.join(directory, entry)
    except Exception as e:
        decky.logger.error(f"[Hydra] Failed to list {directory}: {e}")
    return None


def _iter_steam_shortcuts():
    """Yield (appname, exe) for every non-Steam shortcut, across all Steam users.

    Games launched through Proton don't get their executablePath backfilled in
    the Hydra database, so this is the only reliable way to locate their
    install directory (Steam's shortcuts.vdf already has the resolved path).
    """
    userdata = Path(decky.DECKY_USER_HOME) / ".local" / "share" / "Steam" / "userdata"
    if not userdata.is_dir():
        return
    for user_dir in userdata.iterdir():
        if not user_dir.is_dir() or not user_dir.name.isdigit():
            continue
        shortcuts_path = user_dir / "config" / "shortcuts.vdf"
        if not shortcuts_path.is_file():
            continue
        try:
            data = shortcuts_path.read_bytes()
        except Exception as e:
            decky.logger.error(f"[Hydra] Failed to read shortcuts.vdf: {e}")
            continue

        appname = None
        i = 0
        while i < len(data):
            if data[i] != 0x01:
                i += 1
                continue
            j = i + 1
            while j < len(data) and data[j] != 0:
                j += 1
            key = data[i + 1:j].decode("utf-8", errors="ignore").lower()
            start = j + 1
            end = data.find(b"\x00", start)
            if end == -1:
                break
            value = data[start:end].decode("utf-8", errors="ignore").strip('"')
            if key == "appname":
                appname = value
            elif key == "exe" and appname:
                yield appname, value
                appname = None
            i = end + 1


def find_ini_by_title(title: str):
    if not title:
        return None
    for appname, exe in _iter_steam_shortcuts():
        if appname.strip().lower() == title.strip().lower():
            ini_path = find_ini(exe)
            if ini_path:
                return ini_path
    return None


def get_settings(executable_path: str, title: str):
    ini_path = find_ini(executable_path) or find_ini_by_title(title)
    if not ini_path:
        return None

    try:
        with open(ini_path, "r", encoding="utf-8", errors="ignore") as f:
            lines = f.readlines()
    except Exception as e:
        decky.logger.error(f"[Hydra] Failed to read {ini_path}: {e}")
        return None

    section = None
    user_name = None
    language = None
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            section = stripped[1:-1]
        elif section == "Settings" and stripped.startswith(USERNAME_KEY):
            user_name = stripped[len(USERNAME_KEY):]
        elif section == "Settings" and stripped.startswith(LANGUAGE_KEY):
            language = stripped[len(LANGUAGE_KEY):]

    # steam_emu.ini always has both keys, but bail out if the file doesn't
    # actually match the expected format rather than returning empty values
    if user_name is None and language is None:
        return None

    return {"iniPath": ini_path, "userName": user_name or "", "language": language or ""}


def set_settings(ini_path: str, user_name: str, language: str):
    try:
        with open(ini_path, "r", encoding="utf-8", errors="ignore") as f:
            lines = f.readlines()

        section = None
        found_username = False
        found_language = False
        settings_insert_at = None
        updated_lines = []
        for line in lines:
            stripped = line.strip()
            if stripped.startswith("[") and stripped.endswith("]"):
                if section == "Settings" and settings_insert_at is None:
                    settings_insert_at = len(updated_lines)
                section = stripped[1:-1]
                updated_lines.append(line)
            elif section == "Settings" and stripped.startswith(USERNAME_KEY):
                updated_lines.append(f"{USERNAME_KEY}{user_name}\n")
                found_username = True
            elif section == "Settings" and stripped.startswith(LANGUAGE_KEY):
                updated_lines.append(f"{LANGUAGE_KEY}{language}\n")
                found_language = True
            else:
                updated_lines.append(line)

        # A key may be present in the file but blank (get_settings still
        # surfaces it), so insert whichever one is actually missing instead
        # of silently dropping it
        missing_lines = []
        if not found_username:
            missing_lines.append(f"{USERNAME_KEY}{user_name}\n")
        if not found_language:
            missing_lines.append(f"{LANGUAGE_KEY}{language}\n")
        if missing_lines:
            insert_at = settings_insert_at if settings_insert_at is not None else len(updated_lines)
            if insert_at > 0 and not updated_lines[insert_at - 1].endswith("\n"):
                updated_lines[insert_at - 1] += "\n"
            updated_lines[insert_at:insert_at] = missing_lines

        with open(ini_path, "w", encoding="utf-8") as f:
            f.writelines(updated_lines)

        return {"success": True}
    except Exception as e:
        decky.logger.error(f"[Hydra] set_steam_emu_ini_settings error: {e}")
        return {"success": False, "error": str(e)}
