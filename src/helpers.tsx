export const composeToastLogo = (url: string | null) => {
  if (!url) return null;

  return (
    <img
      src={url}
      style={{ width: "100%", height: "100%", objectFit: "cover" }}
    />
  );
};
