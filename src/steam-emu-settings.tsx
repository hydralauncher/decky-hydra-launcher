import { useCallback, useEffect, useState } from "react";
import { toaster } from "@decky/api";
import { Button, PanelSection, PanelSectionRow, Spinner, TextField } from "@decky/ui";
import { composeToastLogo } from "./helpers";
import { getSteamEmuIniSettings, setSteamEmuIniSettings } from "./events";
import type { Game, SteamEmuIniSettings } from "./api-types";

export interface SteamEmuSettingsProps {
  game: Game;
}

export function SteamEmuSettings({ game }: SteamEmuSettingsProps) {
  const [settings, setSettings] = useState<SteamEmuIniSettings | null>(null);
  const [userName, setUserName] = useState("");
  const [language, setLanguage] = useState("");
  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    setIsLoading(true);
    setSettings(null);

    getSteamEmuIniSettings(game.executablePath, game.title)
      .then((result) => {
        setSettings(result);

        if (result) {
          setUserName(result.userName);
          setLanguage(result.language);
        }
      })
      .finally(() => setIsLoading(false));
  }, [game.executablePath, game.title]);

  const handleDetectLanguage = useCallback(async () => {
    try {
      const detected = await SteamClient.Settings.GetCurrentLanguage();
      setLanguage(detected);
    } catch (error: unknown) {
      console.error(error);
    }
  }, []);

  const handleSave = useCallback(async () => {
    if (!settings) return;

    setIsSaving(true);

    try {
      const result = await setSteamEmuIniSettings(
        settings.iniPath,
        userName,
        language
      );

      if (!result.success) {
        throw new Error(result.error);
      }

      toaster.toast({
        title: "Steam emulator settings saved",
        body: `${userName} · ${language}`,
        logo: composeToastLogo(game.iconUrl),
      });
    } catch (error: unknown) {
      console.error(error);

      toaster.toast({
        title: "Failed to save settings",
        body: "Please check if the game files are correct",
      });
    } finally {
      setIsSaving(false);
    }
  }, [settings, userName, language, game.iconUrl]);

  return (
    <PanelSection title="Steam Emulator">
      {isLoading && (
        <PanelSectionRow>
          <div className="steam-emu-settings__status">
            <Spinner width={15} />
            Looking for a steam_emu.ini file...
          </div>
        </PanelSectionRow>
      )}

      {!isLoading && !settings && (
        <PanelSectionRow>
          <span className="steam-emu-settings__status">
            No Steam emulator detected for this game.
          </span>
        </PanelSectionRow>
      )}

      {!isLoading && settings && (
        <>
          <PanelSectionRow>
            <TextField
              label="Username"
              value={userName}
              onChange={(e) => setUserName(e.target.value)}
            />
          </PanelSectionRow>

          <PanelSectionRow>
            <TextField
              label="Language"
              value={language}
              onChange={(e) => setLanguage(e.target.value)}
            />
          </PanelSectionRow>

          <PanelSectionRow>
            <Button className="steam-emu-settings__button" onClick={handleDetectLanguage}>
              Use my Steam language
            </Button>
          </PanelSectionRow>

          <PanelSectionRow>
            <Button
              className="steam-emu-settings__button"
              onClick={handleSave}
              disabled={isSaving}
            >
              {isSaving ? <Spinner width={15} /> : "Save"}
            </Button>
          </PanelSectionRow>
        </>
      )}
    </PanelSection>
  );
}
