import type { MuonCacheMetrics } from "./types";

type GpuCtx = {
  device: GPUDevice;
  context: GPUCanvasContext;
  linePipeline: GPURenderPipeline;
  triPipeline: GPURenderPipeline;
};

const HISTORY_POINTS = 600;

function append(history: number[], value: number): void {
  history.push(value);
  if (history.length > HISTORY_POINTS) {
    history.shift();
  }
}

function buildLineVertices(
  values: number[],
  color: [number, number, number, number],
  yTop: number,
  yBottom: number,
  min: number,
  max: number
): Float32Array {
  if (values.length < 2) {
    return new Float32Array(0);
  }
  const range = Math.max(1e-9, max - min);
  const verts = new Float32Array(values.length * 6);
  const startX = -0.95;
  const endX = 0.95;
  const step = (endX - startX) / Math.max(1, values.length - 1);

  for (let i = 0; i < values.length; i += 1) {
    const x = startX + step * i;
    const normalized = (values[i] - min) / range;
    const y = yBottom + normalized * (yTop - yBottom);
    const offset = i * 6;
    verts[offset] = x;
    verts[offset + 1] = y;
    verts[offset + 2] = color[0];
    verts[offset + 3] = color[1];
    verts[offset + 4] = color[2];
    verts[offset + 5] = color[3];
  }

  return verts;
}

function buildBarVertices(commandMix: Record<string, number>): Float32Array {
  const entries = Object.entries(commandMix)
    .sort((a, b) => b[1] - a[1])
    .slice(0, 6);

  if (!entries.length) {
    return new Float32Array(0);
  }

  const palette: [number, number, number, number][] = [
    [0.17, 0.67, 0.90, 0.90],
    [0.85, 0.38, 0.20, 0.90],
    [0.30, 0.75, 0.45, 0.90],
    [0.78, 0.68, 0.16, 0.90],
    [0.60, 0.42, 0.86, 0.90],
    [0.93, 0.54, 0.65, 0.90]
  ];

  const maxVal = Math.max(1, ...entries.map(([, value]) => value));
  const left = -0.95;
  const right = 0.95;
  const bottom = -0.95;
  const top = -0.55;
  const width = right - left;
  const slot = width / entries.length;
  const barWidth = slot * 0.7;

  const vertices: number[] = [];

  for (let i = 0; i < entries.length; i += 1) {
    const [, raw] = entries[i];
    const h = ((raw / maxVal) * (top - bottom)) * 0.95;
    const x0 = left + i * slot + (slot - barWidth) * 0.5;
    const x1 = x0 + barWidth;
    const y0 = bottom;
    const y1 = bottom + h;
    const c = palette[i % palette.length];

    vertices.push(
      x0,
      y0,
      c[0],
      c[1],
      c[2],
      c[3],
      x1,
      y0,
      c[0],
      c[1],
      c[2],
      c[3],
      x1,
      y1,
      c[0],
      c[1],
      c[2],
      c[3],
      x0,
      y0,
      c[0],
      c[1],
      c[2],
      c[3],
      x1,
      y1,
      c[0],
      c[1],
      c[2],
      c[3],
      x0,
      y1,
      c[0],
      c[1],
      c[2],
      c[3]
    );
  }

  return new Float32Array(vertices);
}

export class MetricsDashboard {
  private readonly canvas: HTMLCanvasElement;
  private readonly warningEl: HTMLElement;
  private gpu: GpuCtx | null = null;

  private readonly opsHistory: number[] = [];
  private readonly p50History: number[] = [];
  private readonly p95History: number[] = [];
  private readonly p99History: number[] = [];
  private commandMix: Record<string, number> = {};

  private raf: number | ReturnType<typeof setTimeout> = 0;
  private uncapped = false;
  private frameCount = 0;
  private fpsTs = performance.now();
  private fpsValue = 0;

  constructor(canvas: HTMLCanvasElement, warningEl: HTMLElement) {
    this.canvas = canvas;
    this.warningEl = warningEl;
  }

  async init(): Promise<boolean> {
    const nav = navigator as Navigator & { gpu?: GPU };
    if (!nav.gpu) {
      this.warningEl.textContent = "WebGPU not available. Use latest Chrome/Edge and keep hardware acceleration enabled.";
      this.warningEl.classList.remove("hidden");
      return false;
    }

    const adapter = await nav.gpu.requestAdapter();
    if (!adapter) {
      this.warningEl.textContent = "WebGPU adapter not found. The dashboard needs WebGPU in this demo.";
      this.warningEl.classList.remove("hidden");
      return false;
    }

    const device = await adapter.requestDevice();
    const context = this.canvas.getContext("webgpu") as GPUCanvasContext | null;
    if (!context) {
      this.warningEl.textContent = "Failed to get WebGPU canvas context.";
      this.warningEl.classList.remove("hidden");
      return false;
    }

    const format = nav.gpu.getPreferredCanvasFormat();
    context.configure({
      device,
      format,
      alphaMode: "opaque"
    });

    const shader = device.createShaderModule({
      code: `
struct VertexOut {
  @builtin(position) position : vec4<f32>,
  @location(0) color : vec4<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) color: vec4<f32>) -> VertexOut {
  var out: VertexOut;
  out.position = vec4<f32>(pos, 0.0, 1.0);
  out.color = color;
  return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
  return in.color;
}
`
    });

    const vertexLayout: GPUVertexBufferLayout = {
      arrayStride: 24,
      attributes: [
        { shaderLocation: 0, offset: 0, format: "float32x2" },
        { shaderLocation: 1, offset: 8, format: "float32x4" }
      ]
    };

    const layout = device.createPipelineLayout({ bindGroupLayouts: [] });

    const linePipeline = device.createRenderPipeline({
      layout,
      vertex: {
        module: shader,
        entryPoint: "vs_main",
        buffers: [vertexLayout]
      },
      fragment: {
        module: shader,
        entryPoint: "fs_main",
        targets: [{ format }]
      },
      primitive: {
        topology: "line-strip"
      }
    });

    const triPipeline = device.createRenderPipeline({
      layout,
      vertex: {
        module: shader,
        entryPoint: "vs_main",
        buffers: [vertexLayout]
      },
      fragment: {
        module: shader,
        entryPoint: "fs_main",
        targets: [{ format }]
      },
      primitive: {
        topology: "triangle-list"
      }
    });

    this.gpu = { device, context, linePipeline, triPipeline };
    this.startRenderLoop();
    return true;
  }

  pushMetrics(metrics: MuonCacheMetrics): void {
    append(this.opsHistory, metrics.ops_window_1s);
    append(this.p50History, metrics.latency_p50_us);
    append(this.p95History, metrics.latency_p95_us);
    append(this.p99History, metrics.latency_p99_us);
    this.commandMix = metrics.command_mix;
  }

  fps(): number {
    return this.fpsValue;
  }

  setUncapped(uncapped: boolean): void {
    if (this.uncapped === uncapped) {
      return;
    }
    this.uncapped = uncapped;
    this.stopRenderLoop();
    this.startRenderLoop();
  }

  destroy(): void {
    this.stopRenderLoop();
  }

  private stopRenderLoop(): void {
    if (!this.raf) {
      return;
    }
    if (this.uncapped) {
      clearTimeout(this.raf as ReturnType<typeof setTimeout>);
    } else {
      cancelAnimationFrame(this.raf as number);
    }
    this.raf = 0;
  }

  private startRenderLoop(): void {
    const render = (): void => {
      this.frameCount += 1;
      const now = performance.now();
      if (now - this.fpsTs >= 1000) {
        this.fpsValue = Math.round((this.frameCount * 1000) / (now - this.fpsTs));
        this.frameCount = 0;
        this.fpsTs = now;
      }

      this.renderFrame();
      this.raf = this.uncapped
        ? setTimeout(render, 0)
        : requestAnimationFrame(render);
    };

    this.raf = this.uncapped
      ? setTimeout(render, 0)
      : requestAnimationFrame(render);
  }

  private renderFrame(): void {
    if (!this.gpu) {
      return;
    }

    const { device, context, linePipeline, triPipeline } = this.gpu;

    const opsMax = Math.max(10, ...this.opsHistory);
    const latencyMax = Math.max(100, ...this.p99History);

    const opsLine = buildLineVertices(
      this.opsHistory,
      [0.17, 0.67, 0.90, 0.95],
      0.92,
      0.20,
      0,
      opsMax
    );
    const p50Line = buildLineVertices(
      this.p50History,
      [0.95, 0.65, 0.23, 0.95],
      0.10,
      -0.48,
      0,
      latencyMax
    );
    const p95Line = buildLineVertices(
      this.p95History,
      [0.97, 0.33, 0.28, 0.95],
      0.10,
      -0.48,
      0,
      latencyMax
    );
    const p99Line = buildLineVertices(
      this.p99History,
      [0.85, 0.20, 0.55, 0.95],
      0.10,
      -0.48,
      0,
      latencyMax
    );
    const bars = buildBarVertices(this.commandMix);

    const encoder = device.createCommandEncoder();
    const pass = encoder.beginRenderPass({
      colorAttachments: [
        {
          view: context.getCurrentTexture().createView(),
          clearValue: { r: 0.03, g: 0.08, b: 0.12, a: 1 },
          loadOp: "clear",
          storeOp: "store"
        }
      ]
    });

    const drawLine = (verts: Float32Array): void => {
      if (!verts.length) {
        return;
      }
      const buffer = device.createBuffer({
        size: verts.byteLength,
        usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST
      });
      device.queue.writeBuffer(
        buffer,
        0,
        verts.buffer as ArrayBuffer,
        verts.byteOffset,
        verts.byteLength
      );
      pass.setPipeline(linePipeline);
      pass.setVertexBuffer(0, buffer);
      pass.draw(verts.length / 6);
    };

    drawLine(opsLine);
    drawLine(p50Line);
    drawLine(p95Line);
    drawLine(p99Line);

    if (bars.length) {
      const buffer = device.createBuffer({
        size: bars.byteLength,
        usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST
      });
      device.queue.writeBuffer(
        buffer,
        0,
        bars.buffer as ArrayBuffer,
        bars.byteOffset,
        bars.byteLength
      );
      pass.setPipeline(triPipeline);
      pass.setVertexBuffer(0, buffer);
      pass.draw(bars.length / 6);
    }

    pass.end();
    device.queue.submit([encoder.finish()]);
  }
}
