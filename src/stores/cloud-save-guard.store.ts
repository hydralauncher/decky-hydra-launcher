import { create } from "zustand";

interface CloudSaveGuardStore {
  remoteNewerGames: string[];
  flagRemoteNewer: (objectId: string) => void;
  clearRemoteNewer: (objectId: string) => void;
}

// Games whose remote snapshot is newer than the local sync state. Flagged at
// game launch; post-exit sync is suppressed for them until the user restores
// or syncs manually.
export const useCloudSaveGuard = create<CloudSaveGuardStore>((set) => ({
  remoteNewerGames: [],
  flagRemoteNewer: (objectId) =>
    set((state) =>
      state.remoteNewerGames.includes(objectId)
        ? state
        : { remoteNewerGames: [...state.remoteNewerGames, objectId] }
    ),
  clearRemoteNewer: (objectId) =>
    set((state) => ({
      remoteNewerGames: state.remoteNewerGames.filter((id) => id !== objectId),
    })),
}));
