import { create } from "zustand";

export interface Auth {
  accessToken: string;
  refreshToken: string;
  tokenExpirationTimestamp: number;
}

interface AuthStore {
  auth: Auth | null;
  setAuth: (auth: Auth) => void;
}

export const useAuthStore = create<AuthStore>((set) => ({
  auth: null,
  setAuth: (auth) => set({ auth }),
}));
