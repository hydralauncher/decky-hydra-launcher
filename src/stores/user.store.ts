import { create } from "zustand";
import type { User } from "../api-types";

interface UserStore {
  user: User | null;
  hasActiveSubscription: boolean;
  setUser: (user: User) => void;
}

export const useUserStore = create<UserStore>((set) => ({
  user: null,
  hasActiveSubscription: false,
  setUser: (user) => {
    const expiresAt = new Date(user?.subscription?.expiresAt ?? 0);
    const hasActiveSubscription = expiresAt > new Date();

    set({ user, hasActiveSubscription });
  },
}));
