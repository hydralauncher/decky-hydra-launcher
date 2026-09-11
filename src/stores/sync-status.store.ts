import { create } from "zustand";

export interface SyncStatusSnapshot {
  remoteNewer: boolean;
  localDirty: boolean;
  remoteVersion: number | null;
  localVersion: number | null;
  remoteFileCount: number | null;
  remoteTotalBytes: number | null;
  remoteUpdatedAt: string | null;
  localFileCount: number | null;
  localTotalBytes: number | null;
  localUpdatedAt: string | null;
  checkedAt: number;
}

interface SyncStatusStore {
  lastStatus: Record<string, SyncStatusSnapshot>;
  setStatus: (objectId: string, snapshot: SyncStatusSnapshot) => void;
  clearStatus: (objectId: string) => void;
}

export const useSyncStatusStore = create<SyncStatusStore>()((set) => ({
  lastStatus: {},
  setStatus: (objectId, snapshot) =>
    set((state) => ({
      lastStatus: { ...state.lastStatus, [objectId]: snapshot },
    })),
  clearStatus: (objectId) =>
    set((state) => {
      if (!(objectId in state.lastStatus)) return state;
      const next = { ...state.lastStatus };
      delete next[objectId];
      return { lastStatus: next };
    }),
}));
