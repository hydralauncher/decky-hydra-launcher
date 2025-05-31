import { useMemo } from "react";
import { useCurrentGame } from "../stores";

export function usePlaytime() {
  const { elapsedTimeInMillis } = useCurrentGame();

  const { hours, minutes, seconds } = useMemo(() => {
    const totalSeconds = Math.floor(elapsedTimeInMillis / 1000);
    const hours = Math.floor(totalSeconds / 3600);
    const minutes = Math.floor((totalSeconds % 3600) / 60);
    const seconds = totalSeconds % 60;
    return { hours, minutes, seconds };
  }, [elapsedTimeInMillis]);

  return {
    hours: hours.toString().padStart(2, "0"),
    minutes: minutes.toString().padStart(2, "0"),
    seconds: seconds.toString().padStart(2, "0"),
  };
}
