import { DialogButtonPrimary, GamepadButton, Navigation, QuickAccessTab, Spinner } from "@decky/ui";
import type { GamepadEvent } from "@decky/ui";
import { toaster, useQuickAccessVisible } from "@decky/api";
import {
  PiCloud,
  PiCloudArrowDown,
  PiCloudArrowUp,
  PiMonitor,
  PiWarningCircleFill,
} from "react-icons/pi";
import {
  Component,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type ErrorInfo,
  type ReactNode,
} from "react";

import { findGameByShortcutId, invalidatePrePlayCache, markPrePlayVerified, trackBusy } from "./pre-play-sync";
import { logEvent, restoreCloudSave, syncCloudSave } from "./events";
import { attachBlockListeners, detachBlockListeners, disengagePlayBlock, engagePlayBlock, setQamOpen } from "./play-block";
import {
  useAuthStore,
  useCloudSaveGuard,
  useNavigationStore,
  usePlayBlockStore,
  useSyncStatusStore,
  useUserStore,
} from "./stores";
import { composeToastLogo, formatBytes } from "./helpers";

export const SYNC_OVERLAY_ATTR = "data-hydra-sync-overlay";

const qamHookAvailable = typeof useQuickAccessVisible === "function";

const useQamVisible = qamHookAvailable ? useQuickAccessVisible : () => false;

const spinnerAvailable = typeof Spinner !== "undefined";

const navAvailable = () =>
  typeof Navigation !== "undefined" &&
  typeof Navigation.OpenQuickAccessMenu === "function";

class SyncOverlayBoundary extends Component<
  { appid: string; children: ReactNode },
  { failed: boolean }
> {
  constructor(props: { appid: string; children: ReactNode }) {
    super(props);
    this.state = { failed: false };
  }

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    const message =
      error instanceof Error ? `${error.name}: ${error.message}` : String(error);
    logEvent(
      `sync overlay error: ${this.props.appid} ${message.slice(0, 200)} at ${info.componentStack
        ?.split("\n")[1]
        ?.trim()
        .slice(0, 120) ?? "unknown"}`
    );
  }

  render() {
    return this.state.failed ? null : this.props.children;
  }
}

class ActionBoundary extends Component<
  { children: ReactNode; fallback: ReactNode },
  { failed: boolean }
> {
  constructor(props: { children: ReactNode; fallback: ReactNode }) {
    super(props);
    this.state = { failed: false };
  }

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch() {
    logEvent("sync overlay actions fallback: native buttons");
  }

  render() {
    return this.state.failed ? this.props.fallback : this.props.children;
  }
}

const scrimStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  zIndex: 9999,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  backgroundColor: "rgba(0, 0, 0, 0.72)",
  transition: "opacity 200ms ease",
};

const catcherStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  zIndex: 9999,
  backgroundColor: "rgba(0, 0, 0, 0)",
  pointerEvents: "none",
};

const cardStyle: CSSProperties = {
  width: "min(420px, 86vw)",
  padding: "28px 32px",
  borderRadius: "8px",
  backgroundColor: "#1a1d29",
  color: "#dcdedf",
  textAlign: "center",
  boxShadow: "0 12px 48px rgba(0, 0, 0, 0.6)",
  outline: "none",
};

const titleStyle: CSSProperties = {
  margin: "0 0 4px",
  fontSize: "20px",
  fontWeight: 600,
  color: "#ffc53d",
};

const subtitleStyle: CSSProperties = {
  margin: "0 0 20px",
  fontSize: "15px",
  fontWeight: 600,
  color: "#ffc53d",
};

const warningIconStyle: CSSProperties = {
  display: "flex",
  justifyContent: "center",
  margin: "0 0 12px",
  color: "#ffc53d",
};

const fallbackButtonStyle: CSSProperties = {
  width: "100%",
  padding: "10px 12px",
  border: "2px solid #1a9fff",
  borderRadius: "4px",
  backgroundColor: "#0e141b",
  color: "#fff",
  fontSize: "15px",
  fontWeight: 600,
  cursor: "pointer",
};

const choiceLabelStyle: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  gap: "8px",
  width: "100%",
  fontSize: "17px",
  fontWeight: 600,
  textAlign: "center",
};

const versionRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: "10px",
  margin: "0 0 8px",
  fontSize: "14px",
  color: "#dcdedf",
  textAlign: "left",
};

const versionSubStyle: CSSProperties = {
  margin: "-4px 0 8px 32px",
  fontSize: "12px",
  color: "#acb2b8",
  textAlign: "left",
};

const actionsRowStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: "12px",
  marginTop: "20px",
};

const fallbackNoteStyle: CSSProperties = {
  margin: "12px 0 0",
  fontSize: "12px",
  color: "#acb2b8",
};

const choiceButtonStyle: CSSProperties = {
  width: "100%",
};

const formatDateTime = (iso: string | null): string | null => {
  if (!iso) return null;
  const ms = Date.parse(iso);
  if (!Number.isFinite(ms)) return null;
  return new Date(ms).toLocaleString();
};

const ownerDocumentOf = (node: Element | null): Document | null =>
  node?.ownerDocument ?? null;

export const SyncBlockOverlayView = ({ appid }: { appid: string }) => {
  const blockedGames = usePlayBlockStore((state) => state.blockedGames);
  const remoteNewerGames = useCloudSaveGuard((state) => state.remoteNewerGames);
  const lastStatus = useSyncStatusStore((state) => state.lastStatus);
  const checkingStatus = useSyncStatusStore((state) => state.checking);
  const qamVisible = useQamVisible();
  const [visible, setVisible] = useState(false);
  const [resolving, setResolving] = useState<"local" | "remote" | null>(null);
  const cardRef = useRef<HTMLDivElement>(null);
  const scrimRef = useRef<HTMLDivElement>(null);
  const primaryActionRef = useRef<HTMLDivElement>(null);
  const secondaryActionRef = useRef<HTMLDivElement>(null);
  const previousFocus = useRef<Element | null>(null);
  const wasBlocked = useRef(false);

  const game = findGameByShortcutId(appid);
  const blocked = game ? blockedGames.has(game.objectId) : false;
  const conflicted = game ? remoteNewerGames.includes(game.objectId) : false;

  useEffect(() => {
    setQamOpen(qamVisible);
    return () => setQamOpen(false);
  }, [qamVisible]);

  useEffect(() => {
    if (blocked && !wasBlocked.current) {
      wasBlocked.current = true;
      logEvent(
        `sync overlay show: ${game?.objectId ?? appid} conflict=${remoteNewerGames.includes(game?.objectId ?? "")} qamHook=${qamHookAvailable} nav=${navAvailable()} spinner=${spinnerAvailable}`
      );
    }
    if (!blocked) {
      wasBlocked.current = false;
      setVisible(false);
      return;
    }
    previousFocus.current =
      ownerDocumentOf(scrimRef.current)?.activeElement ?? null;
    const timer = setTimeout(() => setVisible(true), 30);
    return () => clearTimeout(timer);
  }, [blocked, appid, game, remoteNewerGames]);

  useEffect(() => {
    const doc = ownerDocumentOf(scrimRef.current);
    if (!blocked || !doc) return;
    attachBlockListeners(doc);
    return () => detachBlockListeners(doc);
  }, [blocked]);

  useEffect(() => {
    if (!blocked || !visible || qamVisible || !conflicted) return;
    const doc = ownerDocumentOf(scrimRef.current);
    try {
      const target = primaryActionRef.current ?? cardRef.current ?? scrimRef.current;
      target?.focus({ preventScroll: true });
    } catch (error) {
      logEvent(
        `sync overlay focus failed: ${error instanceof Error ? error.message : "unknown"}`
      );
      return;
    }
    return () => {
      try {
        const previous = previousFocus.current as HTMLElement | null;
        if (previous && doc?.contains(previous)) previous.focus({ preventScroll: true });
      } catch {
        return;
      }
    };
  }, [blocked, visible, qamVisible, conflicted]);

  if (!blocked || !game || qamVisible) return null;

  if (!conflicted) {
    return (
      <div
        ref={scrimRef}
        data-hydra-sync-overlay=""
        tabIndex={-1}
        style={catcherStyle}
      />
    );
  }

  const snapshot = lastStatus[game.objectId];
  const statusChecking = checkingStatus[game.objectId] ?? false;
  const pendingLabel = statusChecking ? "Checking…" : "unknown";

  const localDetail = [
    snapshot?.localFileCount != null ? `${snapshot.localFileCount} files` : null,
    snapshot?.localTotalBytes != null ? formatBytes(snapshot.localTotalBytes) : null,
  ]
    .filter((part): part is string => part !== null)
    .join(" · ");
  const remoteDetail = [
    snapshot?.remoteFileCount != null ? `${snapshot.remoteFileCount} files` : null,
    snapshot?.remoteTotalBytes != null ? formatBytes(snapshot.remoteTotalBytes) : null,
  ]
    .filter((part): part is string => part !== null)
    .join(" · ");
  const localDate = formatDateTime(snapshot?.localUpdatedAt ?? null);
  const remoteDate = formatDateTime(snapshot?.remoteUpdatedAt ?? null);

  const finishResolve = (title: string, body: string) => {
    useCloudSaveGuard.getState().clearRemoteNewer(game.objectId);
    invalidatePrePlayCache(game.objectId);
    markPrePlayVerified(game.objectId);
    toaster.toast({ title, body, logo: composeToastLogo(game.iconUrl) });
    disengagePlayBlock(game.objectId);
  };

  const openDecisions = () => {
    try {
      useNavigationStore.getState().setRoute({ name: "game", params: { game } });
      Navigation.OpenQuickAccessMenu(QuickAccessTab.Decky);
    } catch (error) {
      logEvent(
        `sync overlay nav failed: ${error instanceof Error ? error.message : "unknown"}`
      );
      toaster.toast({
        title: "Save sync needs a decision",
        body: "Open Hydra → Pending decisions to choose.",
      });
    }
  };

  const resolveConflict = async (side: "local" | "remote") => {
    const { auth } = useAuthStore.getState();
    const { hasActiveSubscription } = useUserStore.getState();
    if (!auth || !hasActiveSubscription || resolving) return;
    logEvent(`conflict resolve start: ${game.objectId} side=${side}`);
    engagePlayBlock(game.objectId);
    setResolving(side);
    try {
      if (side === "local") {
        const result = await trackBusy(
          game.objectId,
          "sync",
          () => syncCloudSave(auth, game.objectId, game.shop, game.winePrefixPath, true, null)
        );
        if (result.auth) useAuthStore.getState().setAuth(result.auth);
        if (!result.ok && result.conflict) {
          toaster.toast({
            title: "Cloud save conflict",
            body: `${game.title}: ${result.conflict.length} file(s) need a per-file choice. Open the Hydra plugin to choose.`,
            logo: composeToastLogo(game.iconUrl),
          });
          openDecisions();
          return;
        }
        finishResolve(
          "Cloud save synced",
          `Uploaded ${result.uploadedFiles} files (${result.skippedFiles} already in the cloud)`
        );
      } else {
        const result = await trackBusy(
          game.objectId,
          "restore",
          () => restoreCloudSave(auth, game.objectId, game.shop, game.winePrefixPath)
        );
        if (result.auth) useAuthStore.getState().setAuth(result.auth);
        if (result.skippedFiles.length === 0) {
          finishResolve(
            "Cloud save restored",
            `Restored ${result.restoredFiles} files`
          );
        } else {
          toaster.toast({
            title: "Cloud save partially restored",
            body: `${game.title}: ${result.skippedFiles.length} files could not be restored.`,
            logo: composeToastLogo(game.iconUrl),
          });
          disengagePlayBlock(game.objectId);
        }
      }
    } catch (error) {
      toaster.toast({
        title: "Failed to resolve conflict",
        body: error instanceof Error ? error.message : "Unknown error",
      });
      disengagePlayBlock(game.objectId);
    } finally {
      setResolving(null);
    }
  };

  const trapDirection = (side: "local" | "cloud") => (evt: GamepadEvent) => {
    const button = evt.detail?.button;
    if (side === "local" && button === GamepadButton.DIR_DOWN) {
      secondaryActionRef.current?.focus({ preventScroll: true });
    } else if (side === "cloud" && button === GamepadButton.DIR_UP) {
      primaryActionRef.current?.focus({ preventScroll: true });
    }
    evt.stopPropagation();
    evt.preventDefault();
  };

  return (
    <div
      ref={scrimRef}
      data-hydra-sync-overlay=""
      style={{ ...scrimStyle, opacity: visible ? 1 : 0 }}
    >
      <div ref={cardRef} tabIndex={-1} style={cardStyle}>
        <div style={warningIconStyle}>
          <PiWarningCircleFill size={28} />
        </div>
        <h2 style={titleStyle}>{game.title}</h2>
        <p style={subtitleStyle}>has conflicting saves</p>
        <div style={versionRowStyle}>
          <PiMonitor size={16} />
          <span>Local {localDetail || pendingLabel}</span>
        </div>
        {localDate ? <p style={versionSubStyle}>Last edited {localDate}</p> : null}
        <div style={versionRowStyle}>
          <PiCloud size={16} />
          <span>Cloud {remoteDetail || pendingLabel}</span>
        </div>
        {remoteDate ? <p style={versionSubStyle}>Last edited {remoteDate}</p> : null}
        <div style={actionsRowStyle}>
          <ActionBoundary
            fallback={
              <>
                <button
                  style={fallbackButtonStyle}
                  disabled={resolving !== null}
                  onClick={() => resolveConflict("local")}
                >
                  <span style={choiceLabelStyle}>
                    <PiCloudArrowUp size={20} />
                    Keep local
                  </span>
                </button>
                <button
                  style={fallbackButtonStyle}
                  disabled={resolving !== null}
                  onClick={() => resolveConflict("remote")}
                >
                  <span style={choiceLabelStyle}>
                    <PiCloudArrowDown size={20} />
                    Keep cloud
                  </span>
                </button>
                <p style={fallbackNoteStyle}>
                  Gamepad selection unavailable — touch works, or decide in the Hydra plugin.
                </p>
              </>
            }
          >
            <DialogButtonPrimary
              ref={primaryActionRef}
              style={choiceButtonStyle}
              disabled={resolving !== null}
              onClick={() => resolveConflict("local")}
              onGamepadDirection={trapDirection("local")}
              onOKActionDescription="Keep local save"
            >
              <span style={choiceLabelStyle}>
                <PiCloudArrowUp size={20} />
                Keep local
              </span>
            </DialogButtonPrimary>
            <DialogButtonPrimary
              ref={secondaryActionRef}
              style={choiceButtonStyle}
              disabled={resolving !== null}
              onClick={() => resolveConflict("remote")}
              onGamepadDirection={trapDirection("cloud")}
              onOKActionDescription="Keep cloud save"
            >
              <span style={choiceLabelStyle}>
                <PiCloudArrowDown size={20} />
                Keep cloud
              </span>
            </DialogButtonPrimary>
          </ActionBoundary>
        </div>
      </div>
    </div>
  );
};

export const SyncBlockOverlay = ({ appid }: { appid: string }) => (
  <SyncOverlayBoundary appid={appid}>
    <SyncBlockOverlayView appid={appid} />
  </SyncOverlayBoundary>
);
