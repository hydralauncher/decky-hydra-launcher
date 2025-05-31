import { Button } from "@decky/ui";
import { QRCode } from "./qr-code";

export function AuthGuide() {
  return (
    <div
      style={{
        display: "flex",
        width: "100%",
        height: "100%",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <h2>
        You must install and connect your Hydra account in order to use this
        plugin.
      </h2>

      <QRCode />

      <h2>
        Learn more at:
        <br /> <Button>https://hy.dra/decky</Button>
      </h2>
    </div>
  );
}
