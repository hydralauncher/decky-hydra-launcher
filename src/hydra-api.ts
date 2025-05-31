import ky, { type BeforeRequestHook } from "ky";
import { useAuthStore } from "./stores";

const API_URL = "https://hydra-api-us-east-1.losbroxas.org";
const ACCESS_TOKEN_EXPIRATION_OFFSET_IN_MS = 1000 * 60 * 5;

const calculateTokenExpirationTimestamp = (expiresIn: number) => {
  return Date.now() + expiresIn * 1000;
};

const refreshToken: BeforeRequestHook = async (request) => {
  const { auth, setAuth } = useAuthStore.getState();

  if (auth) {
    const { tokenExpirationTimestamp, refreshToken } = auth;

    if (tokenExpirationTimestamp) {
      if (
        Number(tokenExpirationTimestamp) <
        Date.now() - ACCESS_TOKEN_EXPIRATION_OFFSET_IN_MS
      ) {
        const { expiresIn, accessToken } = await ky
          .post(`${API_URL}/auth/refresh`, {
            headers: {
              "User-Agent": "Hydra-Decky-Plugin",
            },
            json: {
              refreshToken,
            },
          })
          .json<{
            expiresIn: number;
            accessToken: string;
            refreshToken: string;
          }>();

        setAuth({
          ...auth,
          accessToken,
          tokenExpirationTimestamp:
            calculateTokenExpirationTimestamp(expiresIn),
        });

        request.headers.set("Authorization", `Bearer ${accessToken}`);
      } else {
        request.headers.set("Authorization", `Bearer ${auth.accessToken}`);
      }
    }
  }
};

export const api = ky.create({
  prefixUrl: API_URL,
  headers: {
    "User-Agent": "Hydra-Decky-Plugin",
  },
  hooks: {
    beforeRequest: [refreshToken],
  },
});
