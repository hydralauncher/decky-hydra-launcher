import { create } from "zustand";

export interface SyncStatusSnapshot {
  remoteVersion: number | null;
  localVersion: number | null;
  remoteFileCount: number | null;
  remoteTotalBytes: number | null;
  remoteUpdatedAt: string | null;
  localFileCount: number | null;
  localTotalBytes: number | null;
  localUpdatedAt: string | null;
}

interface SyncStatusStore {
  lastStatus: Record<string, SyncStatusSnapshot>;
  checking: Record<string, boolean>;
  setStatus: (objectId: string, snapshot: SyncStatusSnapshot) => void;
  setChecking: (objectId: string, checking: boolean) => void;
}

export const useSyncStatusStore = create<SyncStatusStore>()((set) => ({
  lastStatus: {},
  checking: {},
  setStatus: (objectId, snapshot) =>
    set((state) => ({
      lastStatus: { ...state.lastStatus, [objectId]: snapshot },
    })),
  setChecking: (objectId, checking) =>
    set((state) => ({
      checking: { ...state.checking, [objectId]: checking },
    })),
}));
