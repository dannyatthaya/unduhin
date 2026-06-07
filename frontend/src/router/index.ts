import { createRouter, createWebHashHistory, type RouteRecordRaw } from "vue-router";

import DownloadsView from "@/views/DownloadsView.vue";
import SettingsLayout from "@/views/settings/SettingsLayout.vue";
import SettingsGeneral from "@/views/settings/SettingsGeneral.vue";
import SettingsCategories from "@/views/settings/SettingsCategories.vue";
import SettingsBehaviour from "@/views/settings/SettingsBehaviour.vue";
import SettingsNetwork from "@/views/settings/SettingsNetwork.vue";
import SettingsTorrent from "@/views/settings/SettingsTorrent.vue";
import SettingsBrowser from "@/views/settings/SettingsBrowser.vue";
import SettingsMedia from "@/views/settings/SettingsMedia.vue";
import SettingsAbout from "@/views/settings/SettingsAbout.vue";

export const SETTINGS_SECTIONS = [
  "general",
  "categories",
  "behaviour",
  "network",
  "torrent",
  "media",
  "browser",
  "about",
] as const;

export type SettingsSectionKey = (typeof SETTINGS_SECTIONS)[number];

const routes: RouteRecordRaw[] = [
  { path: "/", name: "downloads", component: DownloadsView },
  {
    path: "/settings",
    component: SettingsLayout,
    redirect: { name: "settings-general" },
    children: [
      { path: "general", name: "settings-general", component: SettingsGeneral },
      { path: "categories", name: "settings-categories", component: SettingsCategories },
      { path: "behaviour", name: "settings-behaviour", component: SettingsBehaviour },
      { path: "network", name: "settings-network", component: SettingsNetwork },
      { path: "torrent", name: "settings-torrent", component: SettingsTorrent },
      { path: "media", name: "settings-media", component: SettingsMedia },
      { path: "browser", name: "settings-browser", component: SettingsBrowser },
      { path: "about", name: "settings-about", component: SettingsAbout },
    ],
  },
];

export const router = createRouter({
  history: createWebHashHistory(),
  routes,
});
