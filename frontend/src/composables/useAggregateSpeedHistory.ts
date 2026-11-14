// Rolling one-sample-per-second history of the combined download speed,
// for the total-speed sparkline in the downloads toolbar. The store keeps
// per-download histories only, and those arrive at event cadence; sampling
// the aggregate on a fixed tick gives the chart an even time axis.

import { ref, type Ref } from "vue";
import { useIntervalFn } from "@vueuse/core";

import { useDownloadsStore } from "@/stores/downloads";

const SAMPLE_INTERVAL_MS = 1000;
const DEFAULT_CAPACITY = 60;

export function useAggregateSpeedHistory(
  capacity = DEFAULT_CAPACITY,
): Ref<number[]> {
  const downloads = useDownloadsStore();
  const samples = ref<number[]>([]);

  useIntervalFn(() => {
    samples.value = [...samples.value, downloads.aggregateSpeedBps].slice(
      -capacity,
    );
  }, SAMPLE_INTERVAL_MS);

  return samples;
}
