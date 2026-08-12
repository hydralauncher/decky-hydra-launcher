import { GameCloudSaves } from "./game-cloud-saves";
import { SteamEmuSettings } from "./steam-emu-settings";
import type { Game } from "./api-types";

export interface GamePageProps {
  game: Game;
}

export function GamePage({ game }: GamePageProps) {
  return (
    <>
      <GameCloudSaves game={game} />
      <SteamEmuSettings game={game} />
    </>
  );
}
