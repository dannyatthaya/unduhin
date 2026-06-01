// Second Vue entry point. Lives in its own Tauri window
// (`tray-popover`) — a 280×220 borderless always-on-top surface
// rendered just above the system tray. Shares the project's
// `style.css` so theme tokens stay in sync with the main app, but
// runs an isolated Pinia instance and its own event subscription.

import { createApp } from "vue";
import { createPinia } from "pinia";

import "../style.css";
import { i18n } from "../i18n";
import Popover from "./Popover.vue";

const app = createApp(Popover);
app.use(createPinia());
app.use(i18n);
app.mount("#app");
