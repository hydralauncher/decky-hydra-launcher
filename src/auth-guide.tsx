import { Button } from "@decky/ui";
import { QRCodeSVG } from "qrcode.react";

export const GUIDE_URL = "https://hydra.la/decky-guide";

export function AuthGuide() {
  return (
    <div className="auth-guide">
      <h2 className="auth-guide__text">
        You must install and connect your <strong>Hydra</strong> account in
        order to use this plugin.
      </h2>

      <Button className="auth-guide__qr-code">
        <QRCodeSVG
          value={GUIDE_URL}
          width="163px"
          height="163px"
          bgColor="transparent"
          fgColor="#FFFFFF"
        />
      </Button>

      <h2 className="auth-guide__text">
        Learn more at:
        <br />{" "}
        <Button
          className="auth-guide__link"
          onClick={() => {
            window.open(GUIDE_URL, "_blank");
          }}
        >
          {GUIDE_URL}
        </Button>
      </h2>
    </div>
  );
}
