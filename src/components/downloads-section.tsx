import { PanelSection, PanelSectionRow } from "@decky/ui";
import { toaster } from "@decky/api";
import { useEffect, useRef, useState } from "react";
import { useLibraryStore } from "../stores";
import { api } from "../hydra-api";
import { getDownloads, dismissDownload, getSteamShortcutExePaths, updateGameSteamShortcut } from "../events";
import type { Download, GameAssets, Game } from "../api-types";
import { DownloadItem } from "./download-item";

const POLL_INTERVAL_S = 3;

async function imageUrlToBase64(url: string): Promise<{ base64: string; ext: "png" | "jpg" } | null> {
  try {
    const resp = await fetch(url);
    const blob = await resp.blob();
    const ext: "png" | "jpg" = blob.type === "image/png" ? "png" : "jpg";
    const base64 = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onloadend = () => resolve((reader.result as string).split(",")[1]);
      reader.onerror = reject;
      reader.readAsDataURL(blob);
    });
    return { base64, ext };
  } catch {
    return null;
  }
}

function resolveGameTitle(download: Download, library: Pick<Game, "objectId" | "title">[]): string {
  return library.find((g) => g.objectId === download.objectId)?.title
    ?? download.folderName
    ?? download.objectId;
}

export function DownloadsSection() {
  const { library } = useLibraryStore();

  const [steamShortcutPaths, setSteamShortcutPaths] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(true);
  const [activeDownloads, setActiveDownloads] = useState<Download[]>([]);
  const [completedDownloads, setCompletedDownloads] = useState<Download[]>([]);
  const [speeds, setSpeeds] = useState<Map<string, number>>(new Map());

  const prevStatuses = useRef<Map<string, string | null>>(new Map());
  const prevBytes = useRef<Map<string, number>>(new Map());
  const isFirstPoll = useRef(true);

  useEffect(() => {
    getSteamShortcutExePaths()
      .then((paths) => setSteamShortcutPaths(new Set(paths)))
      .catch(() => {});
  }, []);

  useEffect(() => {
    const poll = async () => {
      try {
        const downloads = await getDownloads();

        if (!isFirstPoll.current) {
          const { library: currentLibrary } = useLibraryStore.getState();
          for (const d of downloads) {
            const prev = prevStatuses.current.get(d.objectId);
            if (d.status === "complete" && prev !== null && prev !== "complete") {
              toaster.toast({ title: "Download complete", body: resolveGameTitle(d, currentLibrary) });
            }
          }
        }

        const newSpeeds = new Map<string, number>();
        for (const d of downloads) {
          const prev = prevBytes.current.get(d.objectId);
          if (prev !== undefined && d.status === "active") {
            newSpeeds.set(d.objectId, Math.max(0, d.bytesDownloaded - prev) / POLL_INTERVAL_S);
          }
          prevBytes.current.set(d.objectId, d.bytesDownloaded);
        }
        setSpeeds(newSpeeds);

        isFirstPoll.current = false;
        setLoading(false);
        prevStatuses.current = new Map(downloads.map((d) => [d.objectId, d.status ?? null]));

        setActiveDownloads(
          downloads.filter((d) => d.status === "active" || d.status === "waiting" || d.status === "paused" || d.status === "error" || d.extracting)
        );
        setCompletedDownloads(downloads.filter((d) => d.status === "complete"));
      } catch (_) {}
    };

    poll();
    const interval = setInterval(poll, POLL_INTERVAL_S * 1_000);
    return () => clearInterval(interval);
  }, []);

  const handleDismiss = async (download: Download) => {
    setCompletedDownloads((prev) => prev.filter((d) => d.objectId !== download.objectId));
    try {
      const result = await dismissDownload(download.shop, download.objectId);
      if (!result.success) {
        setCompletedDownloads((prev) => [...prev, download]);
        toaster.toast({ title: "Hydra", body: result.error ?? "Could not delete — close Hydra desktop first" });
      }
    } catch {
      setCompletedDownloads((prev) => [...prev, download]);
    }
  };

  const handleAddToSteam = async (d: Download, executablePath: string, title: string) => {
    const [appId, assets] = await Promise.all([
      SteamClient.Apps.AddShortcut(title, executablePath, "", "") as Promise<number>,
      api.get<GameAssets | null>(`games/steam/${d.objectId}`).json().catch(() => null),
    ]);

    if (assets?.title && appId) {
      SteamClient.Apps.SetShortcutName(appId, assets.title);
    }

    setSteamShortcutPaths((prev) => new Set([...prev, executablePath]));

    updateGameSteamShortcut(d.shop, d.objectId, appId).catch(() => {});

    if (assets && appId) {
      const artworkPairs: [string | null | undefined, number][] = [
        [assets.coverImageUrl, 0],
        [assets.libraryHeroImageUrl, 1],
        [assets.logoImageUrl, 2],
        [assets.libraryImageUrl, 3],
      ];
      for (const [url, assetType] of artworkPairs) {
        if (!url) continue;
        const result = await imageUrlToBase64(url);
        if (result) {
          await SteamClient.Apps.SetCustomArtworkForApp(appId, result.base64, result.ext, assetType).catch(() => {});
        }
      }
    }
  };

  return (
    <PanelSection title="Downloads">
      {loading ? (
        <PanelSectionRow>
          <span className="downloads-placeholder">Loading downloads…</span>
        </PanelSectionRow>
      ) : activeDownloads.length === 0 && completedDownloads.length === 0 ? (
        <PanelSectionRow>
          <span className="downloads-placeholder">No downloads</span>
        </PanelSectionRow>
      ) : (
        <div className="download-list">
          {activeDownloads.map((d) => (
            <DownloadItem
              key={d.objectId}
              download={d}
              title={resolveGameTitle(d, library)}
              iconUrl={library.find((g) => g.objectId === d.objectId)?.iconUrl}
              speed={speeds.get(d.objectId)}
            />
          ))}
          {completedDownloads.map((d) => {
            const game = library.find((g) => g.objectId === d.objectId);
            const executablePath = game?.executablePath ?? null;
            const alreadyInSteam = executablePath ? steamShortcutPaths.has(executablePath) : false;
            return (
              <DownloadItem
                key={d.objectId}
                download={d}
                title={resolveGameTitle(d, library)}
                iconUrl={game?.iconUrl}
                executablePath={alreadyInSteam ? null : executablePath}
                onDismiss={() => handleDismiss(d)}
                onAddToSteam={executablePath ? () => handleAddToSteam(d, executablePath, resolveGameTitle(d, library)) : undefined}
              />
            );
          })}
        </div>
      )}
    </PanelSection>
  );
}
