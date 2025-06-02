import { create } from "zustand";
import type { GameStats } from "../api-types";

interface CurrentGameStore {
  objectId: string | null;
  remoteId: string | null;
  gameStats: GameStats | null;
  startedAt: Date | null;
  elapsedTimeInMillis: number;
  setGameStats: (gameStats: GameStats) => void;
  clearGame: () => void;
  setStartedAt: (startedAt: Date) => void;
  setElapsedTimeInMillis: (elapsedTimeInMillis: number) => void;
  setObjectId: (objectId: string | null) => void;
  setRemoteId: (remoteId: string | null) => void;
}

export const useCurrentGame = create<CurrentGameStore>((set) => ({
  objectId: null,
  remoteId: null,
  gameStats: null,
  startedAt: null,
  elapsedTimeInMillis: 0,
  setGameStats: (gameStats) => set({ gameStats }),
  clearGame: () =>
    set({
      objectId: null,
      remoteId: null,
      gameStats: null,
      startedAt: null,
      elapsedTimeInMillis: 0,
    }),
  setStartedAt: (startedAt) => set({ startedAt }),
  setElapsedTimeInMillis: (elapsedTimeInMillis) => set({ elapsedTimeInMillis }),
  setObjectId: (objectId) => set({ objectId }),
  setRemoteId: (remoteId) => set({ remoteId }),
}));
