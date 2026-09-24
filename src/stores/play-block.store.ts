import { create } from "zustand";

interface PlayBlockStore {
  blockedGames: ReadonlySet<string>;
  engage: (objectId: string) => void;
  disengage: (objectId: string) => void;
}

export const usePlayBlockStore = create<PlayBlockStore>()((set) => ({
  blockedGames: new Set<string>(),
  engage: (objectId) =>
    set((state) => {
      if (state.blockedGames.has(objectId)) return state;
      const next = new Set(state.blockedGames);
      next.add(objectId);
      return { blockedGames: next };
    }),
  disengage: (objectId) =>
    set((state) => {
      if (!state.blockedGames.has(objectId)) return state;
      const next = new Set(state.blockedGames);
      next.delete(objectId);
      return { blockedGames: next };
    }),
}));
