import { create } from "zustand";
import type { Auth } from "../api-types";

interface AuthStore {
  auth: Auth | null;
  setAuth: (auth: Auth) => void;
}

export const useAuthStore = create<AuthStore>((set) => ({
  auth: null,
  setAuth: (auth) => set({ auth }),
}));
