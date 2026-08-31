use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Deserialize;

const MAXIMUM_DEVICES: usize = 8;
const MAXIMUM_SOURCE_BYTES: usize = 65_536;
const MAXIMUM_ENTRY_BYTES: usize = 64;
const MAXIMUM_BUFFERS: usize = 8;
const MAXIMUM_ITEMS: usize = 1_048_576;
const MAXIMUM_WORKGROUPS: u64 = 65_535;

/// One bounded compute request: a checked WGSL module, its entry point, the
/// read-only input buffers bound in order, and the output length. The CPU
/// fallback travels with the artifact so hosts and callers can compare.
#[derive(Deserialize)]
struct ComputeArtifact {
    target: String,
    entry: String,
    source: String,
    workgroups: i64,
    #[serde(default)]
    cpu_fallback: Vec<i64>,
    #[serde(default)]
    buffers: Vec<Vec<i64>>,
    #[serde(default)]
    output_items: i64,
}

/// The bundled WebGPU compute host behind the runtime's `gpu` handle kind.
/// Opening reports honestly when the machine offers no adapter; compute
/// dispatches validated WGSL with bounded storage buffers and returns the
/// read-back values as JSON.
pub struct GpuHost {
    next_handle: AtomicU64,
    devices: Mutex<HashMap<String, (wgpu::Device, wgpu::Queue)>>,
}

impl GpuHost {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            next_handle: AtomicU64::new(1),
            devices: Mutex::new(HashMap::new()),
        })
    }

    pub fn callback(
        self: Arc<Self>,
    ) -> impl Fn(&str, &str) -> Result<String, String> + Send + Sync {
        move |operation, request| self.dispatch(operation, request)
    }

    pub fn dispatch(&self, operation: &str, request: &str) -> Result<String, String> {
        match operation {
            "nivren.handle.open:gpu" => self.open(request),
            "nivren.handle.call:compute" => self.compute(request),
            "nivren.handle.close" => self.close(request),
            _ => Err(format!("unsupported GPU host operation '{operation}'")),
        }
    }

    fn open(&self, configuration: &str) -> Result<String, String> {
        if configuration != "webgpu-wgsl" {
            return Err("GPU host configuration must be 'webgpu-wgsl'".into());
        }
        let mut devices = self
            .devices
            .lock()
            .map_err(|_| "GPU host lock is poisoned")?;
        if devices.len() >= MAXIMUM_DEVICES {
            return Err("GPU host already owns the maximum 8 devices".into());
        }
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .map_err(|error| format!("no GPU adapter is available on this host: {error}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("nivren-gpu-host"),
            ..Default::default()
        }))
        .map_err(|error| format!("cannot open GPU device: {error}"))?;
        let identifier = format!("gpu-{}", self.next_handle.fetch_add(1, Ordering::Relaxed));
        devices.insert(identifier.clone(), (device, queue));
        Ok(identifier)
    }

    fn compute(&self, envelope: &str) -> Result<String, String> {
        let envelope: serde_json::Value = serde_json::from_str(envelope)
            .map_err(|error| format!("invalid GPU handle envelope: {error}"))?;
        let handle = envelope
            .get("handle")
            .and_then(serde_json::Value::as_str)
            .ok_or("GPU handle envelope is missing handle")?;
        let request = envelope
            .get("request")
            .and_then(serde_json::Value::as_str)
            .ok_or("GPU handle envelope is missing request")?;
        let artifact: ComputeArtifact = serde_json::from_str(request)
            .map_err(|error| format!("invalid GPU compute request: {error}"))?;
        validate_artifact(&artifact)?;
        let devices = self
            .devices
            .lock()
            .map_err(|_| "GPU host lock is poisoned")?;
        let (device, queue) = devices
            .get(handle)
            .ok_or("GPU handle is closed or unknown")?;
        let values = run_compute(device, queue, &artifact)?;
        Ok(serde_json::json!({ "values": values }).to_string())
    }

    fn close(&self, handle: &str) -> Result<String, String> {
        self.devices
            .lock()
            .map_err(|_| "GPU host lock is poisoned")?
            .remove(handle)
            .ok_or("GPU handle is closed or unknown")?;
        Ok("closed".into())
    }
}

fn validate_artifact(artifact: &ComputeArtifact) -> Result<(), String> {
    if artifact.target != "webgpu-wgsl" {
        return Err(format!(
            "GPU compute target '{}' is not supported; use 'webgpu-wgsl'",
            artifact.target
        ));
    }
    if artifact.entry.is_empty()
        || artifact.entry.len() > MAXIMUM_ENTRY_BYTES
        || !artifact
            .entry
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err("GPU entry point must be 1 through 64 identifier bytes".into());
    }
    if artifact.source.is_empty() || artifact.source.len() > MAXIMUM_SOURCE_BYTES {
        return Err("GPU WGSL source must contain 1 through 65536 bytes".into());
    }
    if artifact.workgroups < 1 || artifact.workgroups as u64 > MAXIMUM_WORKGROUPS {
        return Err("GPU workgroups must be from 1 through 65535".into());
    }
    if artifact.buffers.is_empty() || artifact.buffers.len() > MAXIMUM_BUFFERS {
        return Err("GPU compute requires 1 through 8 input buffers".into());
    }
    for buffer in &artifact.buffers {
        if buffer.is_empty() || buffer.len() > MAXIMUM_ITEMS {
            return Err("every GPU input buffer holds 1 through 1048576 items".into());
        }
        if buffer.iter().any(|value| i32::try_from(*value).is_err()) {
            return Err("GPU buffer values must fit signed 32-bit integers".into());
        }
    }
    if artifact.output_items < 1 || artifact.output_items as usize > MAXIMUM_ITEMS {
        return Err("GPU output_items must be from 1 through 1048576".into());
    }
    if !artifact.cpu_fallback.is_empty()
        && artifact.cpu_fallback.len() != artifact.output_items as usize
    {
        return Err("GPU cpu_fallback length must match output_items".into());
    }
    Ok(())
}

fn write_buffer(device: &wgpu::Device, values: &[i64], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (values.len() * 4) as u64,
        usage,
        mapped_at_creation: true,
    });
    {
        let mut view = buffer.slice(..).get_mapped_range_mut();
        for (chunk, value) in view.chunks_exact_mut(4).zip(values) {
            chunk.copy_from_slice(&(*value as i32).to_le_bytes());
        }
    }
    buffer.unmap();
    buffer
}

fn run_compute(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    artifact: &ComputeArtifact,
) -> Result<Vec<i64>, String> {
    // Shader and pipeline validation surface as a captured error scope, so a
    // hostile or broken WGSL module fails this request instead of the device.
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("nivren-gpu-kernel"),
        source: wgpu::ShaderSource::Wgsl(artifact.source.as_str().into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("nivren-gpu-pipeline"),
        layout: None,
        module: &module,
        entry_point: Some(&artifact.entry),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    if let Some(error) = pollster::block_on(device.pop_error_scope()) {
        return Err(format!("GPU shader validation failed: {error}"));
    }

    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let output_items = artifact.output_items as usize;
    let inputs = artifact
        .buffers
        .iter()
        .map(|values| {
            write_buffer(
                device,
                values,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            )
        })
        .collect::<Vec<_>>();
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("nivren-gpu-output"),
        size: (output_items * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("nivren-gpu-readback"),
        size: (output_items * 4) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let layout = pipeline.get_bind_group_layout(0);
    let mut entries = Vec::with_capacity(inputs.len() + 1);
    for (index, buffer) in inputs.iter().enumerate() {
        entries.push(wgpu::BindGroupEntry {
            binding: index as u32,
            resource: buffer.as_entire_binding(),
        });
    }
    entries.push(wgpu::BindGroupEntry {
        binding: inputs.len() as u32,
        resource: output.as_entire_binding(),
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("nivren-gpu-bindings"),
        layout: &layout,
        entries: &entries,
    });
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("nivren-gpu-pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(artifact.workgroups as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, (output_items * 4) as u64);
    queue.submit([encoder.finish()]);
    if let Some(error) = pollster::block_on(device.pop_error_scope()) {
        return Err(format!("GPU dispatch validation failed: {error}"));
    }

    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    device
        .poll(wgpu::PollType::Wait)
        .map_err(|error| format!("GPU wait failed: {error}"))?;
    receiver
        .recv()
        .map_err(|_| "GPU readback was abandoned".to_string())?
        .map_err(|error| format!("GPU readback failed: {error}"))?;
    let view = readback.slice(..).get_mapped_range();
    let values = view
        .chunks_exact(4)
        .map(|chunk| i64::from(i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])))
        .collect();
    drop(view);
    readback.unmap();
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::{ComputeArtifact, GpuHost, validate_artifact};

    fn artifact() -> ComputeArtifact {
        ComputeArtifact {
            target: "webgpu-wgsl".into(),
            entry: "add".into(),
            source: "@group(0) @binding(0) var<storage, read> left: array<i32>; \
                     @group(0) @binding(1) var<storage, read> right: array<i32>; \
                     @group(0) @binding(2) var<storage, read_write> output: array<i32>; \
                     @compute @workgroup_size(64) fn add(@builtin(global_invocation_id) id: vec3<u32>) { \
                     let index = id.x; if (index < arrayLength(&output)) { \
                     output[index] = left[index] + right[index]; } }"
                .into(),
            workgroups: 1,
            cpu_fallback: vec![11, 22, 33, 44],
            buffers: vec![vec![1, 2, 3, 4], vec![10, 20, 30, 40]],
            output_items: 4,
        }
    }

    #[test]
    fn artifacts_are_validated_before_any_device_work() {
        let mut bad_target = artifact();
        bad_target.target = "cuda".into();
        assert!(validate_artifact(&bad_target).is_err());
        let mut bad_entry = artifact();
        bad_entry.entry = "no spaces".into();
        assert!(validate_artifact(&bad_entry).is_err());
        let mut bad_buffers = artifact();
        bad_buffers.buffers.clear();
        assert!(validate_artifact(&bad_buffers).is_err());
        let mut bad_values = artifact();
        bad_values.buffers[0][0] = i64::MAX;
        assert!(validate_artifact(&bad_values).is_err());
        assert!(validate_artifact(&artifact()).is_ok());
    }

    #[test]
    fn open_reports_the_adapter_matrix_honestly_and_computes_when_present() {
        let host = GpuHost::new();
        assert!(host.dispatch("nivren.handle.open:gpu", "metal").is_err());
        match host.dispatch("nivren.handle.open:gpu", "webgpu-wgsl") {
            Ok(handle) => {
                let request = serde_json::json!({
                    "target": "webgpu-wgsl",
                    "entry": "add",
                    "source": artifact().source,
                    "workgroups": 1,
                    "cpu_fallback": [11, 22, 33, 44],
                    "buffers": [[1, 2, 3, 4], [10, 20, 30, 40]],
                    "output_items": 4,
                })
                .to_string();
                let envelope =
                    serde_json::json!({ "handle": &handle, "request": request }).to_string();
                let response = host
                    .dispatch("nivren.handle.call:compute", &envelope)
                    .unwrap();
                let decoded: serde_json::Value = serde_json::from_str(&response).unwrap();
                assert_eq!(decoded["values"], serde_json::json!([11, 22, 33, 44]));
                host.dispatch("nivren.handle.close", &handle).unwrap();
            }
            Err(message) => {
                assert!(
                    message.contains("no GPU adapter is available on this host"),
                    "unexpected GPU failure: {message}"
                );
            }
        }
    }
}
