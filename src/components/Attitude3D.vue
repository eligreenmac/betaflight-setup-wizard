<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// Live 3D quad for the orientation check. three.js is loaded on first use (separate chunk,
// bundled from npm, works offline). Emits `fallback` when WebGL is unavailable or the
// renderer can't start, so the parent shows the 2D drawings instead (also the jsdom path).
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { prefersReducedMotion, sceneQuat, webglAvailable } from "../wizard/attitude3d";
import type { Euler } from "../wizard/orientation";
import type { QuadView } from "./quad3d";

const props = defineProps<{ attitude: Euler | null; label: string }>();
const emit = defineEmits<{ fallback: []; ready: [] }>();

const canvas = ref<HTMLCanvasElement | null>(null);
const box = ref<HTMLDivElement | null>(null);
let view: QuadView | null = null;
let ro: ResizeObserver | null = null;
let mql: MediaQueryList | null = null;
let unmounted = false;
const onMotionPref = (e: MediaQueryListEvent) => view?.setReducedMotion(e.matches);

function cssColor(name: string, fallback: string): string {
  const v = box.value ? getComputedStyle(box.value).getPropertyValue(name).trim() : "";
  return v || fallback;
}

function push(): void {
  if (view && props.attitude) view.setTarget(sceneQuat(props.attitude));
}

onMounted(async () => {
  if (!webglAvailable()) return emit("fallback");
  try {
    const { createQuadView } = await import("./quad3d");
    if (unmounted || !canvas.value) return;
    view = createQuadView(
      canvas.value,
      { frame: cssColor("--text", "#222222"), accent: cssColor("--accent", "#f39c12"), props: cssColor("--muted", "#5b6573") },
      prefersReducedMotion(),
    );
  } catch {
    view = null;
    return emit("fallback");
  }
  const size = () => {
    const r = box.value?.getBoundingClientRect();
    if (r) view?.resize(Math.round(r.width), Math.round(r.height));
  };
  size();
  if (typeof ResizeObserver !== "undefined" && box.value) {
    ro = new ResizeObserver(size);
    ro.observe(box.value);
  }
  if (typeof window.matchMedia === "function") {
    mql = window.matchMedia("(prefers-reduced-motion: reduce)");
    mql.addEventListener?.("change", onMotionPref);
  }
  push();
  emit("ready");
});

watch(() => props.attitude, push);

onBeforeUnmount(() => {
  unmounted = true;
  ro?.disconnect();
  mql?.removeEventListener?.("change", onMotionPref);
  view?.dispose();
  view = null;
});
</script>

<template>
  <div ref="box" class="attitude3d" data-test="attitude-3d">
    <canvas ref="canvas" role="img" :aria-label="label" />
  </div>
</template>
