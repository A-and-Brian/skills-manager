import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import * as api from "../lib/tauri";

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/**
 * GitHub device-flow sign-in: shows the user code, opens the verification
 * page and polls until GitHub connects, the code expires or the flow is
 * cancelled (explicitly or by unmounting the caller).
 */
export function useGithubDeviceFlow() {
  const [deviceInfo, setDeviceInfo] = useState<api.GithubDeviceFlowStart | null>(null);
  const deviceCancelRef = useRef(false);

  // Abandon an in-flight device-flow poll loop when leaving the page.
  useEffect(() => () => {
    deviceCancelRef.current = true;
  }, []);

  /** Resolves with the connect result, `"expired"` when the code ran out, or
   * `null` when cancelled. Throws on start or poll failure. */
  const runDeviceFlow = async (
    repoName: string,
  ): Promise<api.GithubBackupConnectResult | "expired" | null> => {
    deviceCancelRef.current = false;
    try {
      const info = await api.githubDeviceFlowStart();
      setDeviceInfo(info);
      void openUrl(info.verification_uri);

      let intervalSec = Math.max(info.interval, 5);
      const deadline = Date.now() + info.expires_in * 1000;
      while (!deviceCancelRef.current && Date.now() < deadline) {
        await sleep(intervalSec * 1000);
        if (deviceCancelRef.current) return null;
        const poll = await api.githubDeviceFlowPoll(info.device_code, repoName);
        if (poll.status === "slow_down") {
          intervalSec += 5;
          continue;
        }
        if (poll.status === "connected" && poll.result) {
          setDeviceInfo(null);
          return poll.result;
        }
        // "pending" → keep polling.
      }
      return deviceCancelRef.current ? null : "expired";
    } finally {
      setDeviceInfo(null);
    }
  };

  const cancelDeviceFlow = () => {
    deviceCancelRef.current = true;
    setDeviceInfo(null);
  };

  return { deviceInfo, runDeviceFlow, cancelDeviceFlow };
}
