//! Blending the game's way. The GameCube draws into an 8-bit frame buffer
//! holding gamma-encoded colour and blends there: an additive flame adds its
//! gamma values, a blob shadow darkens gamma values. Blending linear light
//! instead (Bevy's sRGB target) turns dim additive texels to almost nothing
//! — the tower's torch flames (`P_TORCH`, texels ≤ 0.3) became faint glows —
//! and makes dark translucent layers too light (`docs/rendering.md`,
//! "Colour space").
//!
//! So the 3D camera renders into a float target, the level shader writes
//! gamma-space colour, every blend works on gamma values as the hardware's
//! does, and this pass — after the main passes, before the UI and the final
//! blit — turns the finished picture into linear light for the sRGB output.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::core_3d::graph::{Core3d, Node3d};
use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::render_graph::{NodeRunError, RenderGraphContext, RenderGraphExt, RenderLabel, ViewNode, ViewNodeRunner};
use bevy::render::render_resource::binding_types::{sampler, texture_2d};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId, ColorTargetState,
    ColorWrites, FragmentState, Operations, PipelineCache, RenderPassColorAttachment, RenderPassDescriptor,
    RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages, TextureSampleType,
};
use bevy::render::renderer::{RenderContext, RenderDevice};
use bevy::render::view::ViewTarget;
use bevy::render::{RenderApp, RenderStartup};

pub struct GammaBlendPlugin;

impl Plugin for GammaBlendPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "gamma.wgsl");
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_render_graph_node::<ViewNodeRunner<DecodeNode>>(Core3d, DecodeLabel)
            .add_render_graph_edges(Core3d, (Node3d::Tonemapping, DecodeLabel, Node3d::EndMainPassPostProcessing));
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct DecodeLabel;

#[derive(Default)]
struct DecodeNode;

impl ViewNode for DecodeNode {
    type ViewQuery = &'static ViewTarget;

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        target: QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        // Only the float target holds gamma values (the 3D camera's).
        if !target.is_hdr() {
            return Ok(());
        }
        let decode = world.resource::<DecodePipeline>();
        let pipeline_cache = world.resource::<PipelineCache>();
        let Some(pipeline) = pipeline_cache.get_render_pipeline(decode.pipeline) else { return Ok(()) };
        let post = target.post_process_write();
        let bind_group = render_context.render_device().create_bind_group(
            "gamma_decode_bind_group",
            &pipeline_cache.get_bind_group_layout(&decode.layout),
            &BindGroupEntries::sequential((post.source, &decode.sampler)),
        );
        let pass = RenderPassDescriptor {
            label: Some("gamma decode"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: post.destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        };
        let mut render_pass = render_context.command_encoder().begin_render_pass(&pass);
        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..3, 0..1);
        Ok(())
    }
}

#[derive(Resource)]
struct DecodePipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    pipeline: CachedRenderPipelineId,
}

fn init_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "gamma_decode_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (texture_2d(TextureSampleType::Float { filterable: false }), sampler(SamplerBindingType::NonFiltering)),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());
    let pipeline = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("gamma decode".into()),
        layout: vec![layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: load_embedded_asset!(asset_server.as_ref(), "gamma.wgsl"),
            targets: vec![Some(ColorTargetState {
                format: ViewTarget::TEXTURE_FORMAT_HDR,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(DecodePipeline { layout, sampler, pipeline });
}
