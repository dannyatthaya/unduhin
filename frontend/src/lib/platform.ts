/**
 * Which desktop platform this build targets.
 *
 * The single place the frontend is allowed to branch on platform. Every
 * other module imports these flags rather than sniffing the user agent or
 * calling a Tauri plugin, so there is exactly one thing to change if the
 * detection mechanism ever needs to.
 *
 * `__PLATFORM__` is a Vite `define` carrying Node's `process.platform` from
 * build time (see `vite.config.ts`). That is safe because Tauri builds the
 * frontend and the binary on the same machine, and being a constant means
 * no first-paint flash of the wrong window chrome.
 */

declare const __PLATFORM__: string;

/** Node's platform string for the machine this bundle was built on. */
export const platform: string = __PLATFORM__;

/** macOS: native traffic lights, menu bar, and Cmd-based shortcuts. */
export const isMacOS: boolean = __PLATFORM__ === "darwin";

/** Windows: custom title bar with our own minimize/maximize/close. */
export const isWindows: boolean = __PLATFORM__ === "win32";
