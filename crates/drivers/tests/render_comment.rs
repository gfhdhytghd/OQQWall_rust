use oqqwall_rust_core::{Draft, DraftBlock};
use oqqwall_rust_drivers::renderer::{
    RenderPreviewHeader, RendererRuntimeConfig, render_preview_png,
};

#[test]
fn moderator_comment_is_visible_in_rendered_png() {
    let config = RendererRuntimeConfig::default();
    let mut draft = Draft {
        blocks: vec![DraftBlock::Paragraph {
            text: "投稿正文".into(),
        }],
    };
    let original = render_preview_png(&draft, RenderPreviewHeader::default(), &config).unwrap();
    draft.blocks.push(DraftBlock::Comment {
        text: "已核实，多行评论\n第二行说明".into(),
    });
    let commented = render_preview_png(&draft, RenderPreviewHeader::default(), &config).unwrap();
    let original = image::load_from_memory(&original).unwrap();
    let commented = image::load_from_memory(&commented).unwrap();
    assert_eq!(original.width(), commented.width());
    assert_ne!(
        commented.to_rgba8(),
        original.to_rgba8(),
        "comment must change rendered pixels"
    );
    // Comments use the same text renderer with an explicit label, rather than disappearing.
    let labeled = Draft {
        blocks: vec![
            draft.blocks[0].clone(),
            DraftBlock::Paragraph {
                text: "[评论] 已核实，多行评论\n第二行说明".into(),
            },
        ],
    };
    let labeled = render_preview_png(&labeled, RenderPreviewHeader::default(), &config).unwrap();
    assert_eq!(
        commented.to_rgba8(),
        image::load_from_memory(&labeled).unwrap().to_rgba8()
    );
}
