import { toaster } from "@decky/api";

import type { FriendRequest } from "../../generated/envelope";
import { api } from "../../hydra-api";
import { composeToastLogo } from "../../helpers";
import type { UserProfile } from "./types";

export const friendRequestEvent = async (payload: FriendRequest) => {
  const user = await api.get<UserProfile>(`users/${payload.senderId}`).json();

  if (user) {
    toaster.toast({
      title: "Friend request",
      body: `${user.displayName} wants to be your friend`,
      logo: composeToastLogo(user.profileImageUrl),
    });
  }
};
