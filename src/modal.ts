import { showModal } from "@decky/ui";
import type { ReactNode } from "react";
import type { ShowModalResult } from "@decky/ui";

let activeModal: ShowModalResult | null = null;

export const showSingleModal = (node: ReactNode): ShowModalResult => {
  closeActiveModal();
  activeModal = showModal(node);
  return activeModal;
};

export const closeActiveModal = () => {
  try {
    activeModal?.Close();
  } catch {
    return;
  } finally {
    activeModal = null;
  }
};
