import { useState } from "react";

interface GameIconProps {
  src: string | null | undefined;
  alt?: string;
  size?: number;
}

export function GameIcon({ src, alt = "", size = 30 }: GameIconProps) {
  const [failed, setFailed] = useState(false);

  if (!src || failed) {
    return (
      <div
        className="game-icon-placeholder"
        style={{ width: size, height: size, borderRadius: 8, flexShrink: 0 }}
        title={alt}
      >
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          width={size * 0.55}
          height={size * 0.55}
        >
          <rect x="3" y="3" width="18" height="18" rx="2" />
          <path d="M3 9l4-4 4 4 4-4 4 4" />
          <circle cx="8.5" cy="14.5" r="1.5" />
        </svg>
      </div>
    );
  }

  return (
    <img
      src={src}
      alt={alt}
      width={size}
      height={size}
      style={{ borderRadius: 8, objectFit: "cover", flexShrink: 0 }}
      onError={() => setFailed(true)}
    />
  );
}
