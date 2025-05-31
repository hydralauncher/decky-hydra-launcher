import { toaster } from "@decky/api";

import type { FriendGameSession } from "../../generated/envelope";
import { api } from "../../hydra-api";
import { GameStats } from "../../stores/current-game.store";
import { composeToastLogo } from "../../helpers";
import type { UserProfile } from "./types";

export const friendGameSessionEvent = async (payload: FriendGameSession) => {
  const [friend, gameStats] = await Promise.all([
    api.get<UserProfile>(`users/${payload.friendId}`).json(),
    api
      .get<GameStats>(`games/stats?objectId=${payload.objectId}&shop=steam`)
      .json(),
  ]);

  if (friend && gameStats) {
    toaster.toast({
      title: `${friend.displayName} started playing`,
      body: gameStats.assets.title,
      logo: composeToastLogo(friend.profileImageUrl),
    });
  }
};
