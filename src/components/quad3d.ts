// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * three.js scene for the live orientation check: a simple procedural quad (frame, 4 arms,
 * motors, props, a nose arrow and a front camera block), no model file to fetch or license.
 * Loaded lazily (dynamic import from Attitude3D.vue) so three.js is a separate chunk and the
 * main bundle stays small. three.js is MIT-licensed (see THIRD_PARTY_NOTICES.md).
 *
 * The scene is viewed from behind and above (like Betaflight Configurator): +X right,
 * +Y up, -Z forward. Rotations come in as quaternions from wizard/attitude3d.ts.
 */
import {
  AmbientLight,
  BoxGeometry,
  Color,
  ConeGeometry,
  CylinderGeometry,
  DirectionalLight,
  GridHelper,
  Group,
  Mesh,
  MeshLambertMaterial,
  PerspectiveCamera,
  Quaternion,
  Scene,
  WebGLRenderer,
  type BufferGeometry,
  type Material,
} from "three";
import { smoothingStep, type Quat } from "../wizard/attitude3d";

export interface QuadColors {
  frame: string;
  accent: string;
  props: string;
}

export interface QuadView {
  /** New attitude sample (scene quaternion). */
  setTarget(q: Quat): void;
  setReducedMotion(reduced: boolean): void;
  resize(width: number, height: number): void;
  dispose(): void;
}

function buildQuad(colors: QuadColors): { root: Group; geometries: BufferGeometry[]; materials: Material[] } {
  const geometries: BufferGeometry[] = [];
  const materials: Material[] = [];
  const geo = <G extends BufferGeometry>(g: G): G => (geometries.push(g), g);
  const mat = (color: string, opts: { transparent?: boolean; opacity?: number } = {}) => {
    const m = new MeshLambertMaterial({ color: new Color(color), ...opts });
    materials.push(m);
    return m;
  };
  const frameMat = mat(colors.frame);
  const accentMat = mat(colors.accent);
  const propMat = mat(colors.props, { transparent: true, opacity: 0.45 });

  const root = new Group();
  // Centre plate (FC stack).
  root.add(new Mesh(geo(new BoxGeometry(0.9, 0.18, 1.1)), frameMat));
  // Arms in an X, motors and props at the tips.
  const arm = geo(new BoxGeometry(0.16, 0.08, 1.7));
  const motor = geo(new CylinderGeometry(0.16, 0.16, 0.2, 20));
  const prop = geo(new CylinderGeometry(0.62, 0.62, 0.02, 32));
  for (const [sx, sz] of [[1, -1], [-1, -1], [1, 1], [-1, 1]] as const) {
    const a = new Mesh(arm, frameMat);
    a.position.set(sx * 0.55, 0, sz * 0.55);
    a.rotation.y = sx * sz > 0 ? Math.PI / 4 : -Math.PI / 4;
    root.add(a);
    const m = new Mesh(motor, frameMat);
    m.position.set(sx * 1.1, 0.12, sz * 1.1);
    root.add(m);
    // Front props in the accent colour, like coloured front props on a real quad.
    const p = new Mesh(prop, sz < 0 ? accentMat : propMat);
    p.position.set(sx * 1.1, 0.25, sz * 1.1);
    root.add(p);
  }
  // Nose arrow (points forward = -Z) and a front camera block.
  const arrow = new Mesh(geo(new ConeGeometry(0.22, 0.6, 24)), accentMat);
  arrow.rotation.x = -Math.PI / 2;
  arrow.position.set(0, 0.2, -0.95);
  root.add(arrow);
  const cam = new Mesh(geo(new BoxGeometry(0.34, 0.3, 0.24)), accentMat);
  cam.position.set(0, 0.06, -0.62);
  root.add(cam);
  return { root, geometries, materials };
}

export function createQuadView(canvas: HTMLCanvasElement, colors: QuadColors, reducedMotion: boolean): QuadView {
  const renderer = new WebGLRenderer({ canvas, antialias: true, alpha: true });
  renderer.setPixelRatio(Math.min(2, globalThis.devicePixelRatio || 1));
  const scene = new Scene();
  const camera = new PerspectiveCamera(40, 1, 0.1, 50);
  camera.position.set(0, 3.8, 3.6);
  camera.lookAt(0, 0, 0);
  scene.add(new AmbientLight(0xffffff, 1.6));
  const sun = new DirectionalLight(0xffffff, 1.8);
  sun.position.set(2, 5, 3);
  scene.add(sun);

  const { root, geometries, materials } = buildQuad(colors);
  scene.add(root);
  // A faint, fixed floor grid: the level reference the quad tilts against.
  const grid = new GridHelper(8, 16, new Color(colors.props), new Color(colors.props));
  grid.position.y = -0.9;
  const gridMat = grid.material as Material;
  gridMat.transparent = true;
  gridMat.opacity = 0.25;
  scene.add(grid);

  const target = new Quaternion();
  let reduced = reducedMotion;
  let raf = 0;
  let last = 0;
  let disposed = false;

  const render = () => renderer.render(scene, camera);
  const frame = (now: number) => {
    raf = 0;
    if (disposed) return;
    const dt = last ? now - last : 16;
    last = now;
    root.quaternion.slerp(target, smoothingStep(dt, reduced));
    render();
    // Keep animating only while still catching up (no busy loop when the quad is still).
    if (root.quaternion.angleTo(target) > 0.002) raf = requestAnimationFrame(frame);
    else last = 0;
  };
  const kick = () => {
    if (!raf && !disposed) raf = requestAnimationFrame(frame);
  };

  render();
  return {
    setTarget(q) {
      target.set(q.x, q.y, q.z, q.w).normalize();
      if (reduced) {
        root.quaternion.copy(target);
        render();
      } else kick();
    },
    setReducedMotion(r) {
      reduced = r;
    },
    resize(width, height) {
      if (width <= 0 || height <= 0) return;
      renderer.setSize(width, height, false);
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
      render();
    },
    dispose() {
      disposed = true;
      if (raf) cancelAnimationFrame(raf);
      geometries.forEach((g) => g.dispose());
      grid.geometry.dispose();
      gridMat.dispose();
      materials.forEach((m) => m.dispose());
      renderer.dispose();
    },
  };
}
