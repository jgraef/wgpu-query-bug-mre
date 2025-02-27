use std::time::Instant;

use clap::Parser;
use wgpu::RenderPassTimestampWrites;

#[derive(Debug, Parser)]
struct Args {
    #[clap(long)]
    query_stats: bool,

    #[clap(long)]
    query_times: bool,

    #[clap(long)]
    pass_times: bool,

    #[clap(long)]
    pass_stats: bool,
}

fn main() {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt::init();

    // parse command line flags to know which queries to do
    let args = Args::parse();

    // setup wgpu::{Instance, Device, Queue}
    let backend = Backend::new();

    // setup pipeline statistics and timestamps queries
    let pipeline_statistics_query_set =
        backend.device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("pipeline statistics"),
            ty: wgpu::QueryType::PipelineStatistics(wgpu::PipelineStatisticsTypes::all()),
            count: 1,
        });
    let timestamps_query_set = backend.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("timestamps"),
        ty: wgpu::QueryType::Timestamp,
        count: 3,
    });

    // buffer for query resolution
    let buffer = backend.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT * 2,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    if args.pass_stats || args.pass_times {
        // do a render pass with timestamp writes and a pipeline statistics query
        do_render_pass(
            &backend,
            args.pass_times.then_some(&timestamps_query_set),
            args.pass_stats.then_some(&pipeline_statistics_query_set),
        );
    }

    // resolve queries
    let mut command_encoder = backend.device.create_command_encoder(&Default::default());
    if args.query_stats {
        tracing::debug!("resolving pipeline statistics");
        command_encoder.resolve_query_set(&pipeline_statistics_query_set, 0..1, &buffer, 0);
    }
    if args.query_times {
        tracing::debug!("resolving timestamps");
        command_encoder.resolve_query_set(
            &timestamps_query_set,
            0..1,
            &buffer,
            wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT,
        );
    }

    backend.submit_and_wait(command_encoder);
}

#[derive(Clone, Debug)]
pub struct Backend {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Backend {
    pub fn new() -> Self {
        let instance_flags = wgpu::InstanceFlags::from_build_config().with_env();
        tracing::debug!(?instance_flags);

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            ..Default::default()
        }))
        .expect("no adapter");

        let adapter_info = adapter.get_info();
        tracing::debug!("adapter: {adapter_info:#?}");

        let features = wgpu::Features::TIMESTAMP_QUERY
            | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
            | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES
            | wgpu::Features::PIPELINE_STATISTICS_QUERY;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::default() | features,
                ..Default::default()
            },
            None,
        ))
        .expect("could not open device");

        device.on_uncaptured_error(Box::new(|error| {
            tracing::error!(%error, "uncaptured wgpu error");
            panic!("uncaptured wgpu error: {error}");
        }));

        Self { device, queue }
    }

    pub fn submit_and_wait(&self, command_encoder: wgpu::CommandEncoder) {
        let submission_index = self.queue.submit([command_encoder.finish()]);
        tracing::debug!("waiting for commands to execute");
        let start = Instant::now();
        self.device.poll(wgpu::Maintain::wait_for(submission_index));
        let time = start.elapsed();
        tracing::debug!("finished in {} ms", time.as_millis());
    }
}

fn do_render_pass(
    backend: &Backend,
    timestamp_query: Option<&wgpu::QuerySet>,
    pipeline_statistics_query: Option<&wgpu::QuerySet>,
) {
    tracing::debug!("render pass");

    let mut command_encoder =
        backend
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render pass"),
            });

    // wgpu doesn't let use do a render pass without any attachments, so we will
    // attach a texture
    let texture = backend.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("render target"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());

    let mut render_pass = command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("render pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: timestamp_query.map(|timestamp_query| {
            RenderPassTimestampWrites {
                query_set: timestamp_query,
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index: Some(1),
            }
        }),
        occlusion_query_set: None,
    });

    if let Some(pipeline_statistics_query) = pipeline_statistics_query {
        render_pass.begin_pipeline_statistics_query(&pipeline_statistics_query, 0);
        render_pass.end_pipeline_statistics_query();
    }

    drop(render_pass);
    backend.submit_and_wait(command_encoder);
}
