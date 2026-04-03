import { DialogButton, type DialogButtonProps } from "@decky/ui";

export function Button({ className, ...props }: DialogButtonProps) {
  return (
    <>
      <style>{`
        .hydra-btn:focus,
        .hydra-btn:hover {
          border: 2px solid #fff !important;
        }
      `}</style>
      <DialogButton {...props} className={`hydra-btn${className ? ` ${className}` : ""}`} />
    </>
  );
}
