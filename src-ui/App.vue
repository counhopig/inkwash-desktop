<script setup lang="ts">
import { ref, onMounted } from "vue";
import Sidebar, { type PageId } from "./components/Sidebar.vue";
import TopBar from "./components/TopBar.vue";
import OverviewPage from "./pages/OverviewPage.vue";
import DevicePage from "./pages/DevicePage.vue";
import ContentPage from "./pages/ContentPage.vue";
import LogsPage from "./pages/LogsPage.vue";
import { useDeviceStore } from "./stores/device";
import { useServerStore } from "./stores/server";
import { useLogsStore } from "./stores/logs";

const active = ref<PageId>("overview");

const device = useDeviceStore();
const server = useServerStore();
const logs = useLogsStore();

onMounted(async () => {
  const [, serverReady] = await Promise.all([
    Promise.all([device.bootstrap(), logs.bootstrap()]),
    server.bootstrap(),
  ]);
  if (serverReady) await server.refreshDevices();
});
</script>

<template>
  <div class="app-root">
    <div class="topbar-slot">
      <TopBar />
    </div>
    <div class="sidebar-slot">
      <Sidebar :active="active" @navigate="(p) => (active = p)" />
    </div>
    <main class="main-slot">
      <OverviewPage v-if="active === 'overview'" @navigate="(p) => (active = p)" />
      <DevicePage v-else-if="active === 'device'" />
      <ContentPage v-else-if="active === 'content'" />
      <LogsPage v-else-if="active === 'logs'" />
    </main>
  </div>
</template>
