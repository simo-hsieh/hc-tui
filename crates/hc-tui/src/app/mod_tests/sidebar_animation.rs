use std::time::{Duration, Instant};

use super::*;
use crate::app::sidebar_anim::SIDEBAR_ANIMATION_DURATION;
use crate::config::SidebarCollapsedModeConfig;
use crate::test_support::TestDir;

fn draw_at(app: &mut App, term: &mut Terminal<TestBackend>, now: Instant) {
    app.sched.mark_dirty(now);
    app.maybe_draw(term, now).unwrap();
}

#[tokio::test]
async fn collapse_and_expand_animate_width_and_defer_pane_resize_until_settled() {
    let mut app = app_with_spaces(&[("t1", "work")]);
    app.open_attach = Arc::new(|_| Box::pin(std::future::pending()));
    app.pane = pane::Pane::new(app.pane_tx.clone(), app.open_attach.clone());
    let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
    app.focus_pane("t1".into());
    app.sync_observers();
    app.pending_actions = 0;

    let t0 = Instant::now();
    draw_at(&mut app, &mut term, t0);
    let expanded_sidebar = app.sidebar_width;
    let initial_lease = app.size_leases["t1"].allocated_size();
    assert_eq!(app.hit_map.sidebar_toggle.x, expanded_sidebar - 2);

    // Toggle to collapsed: width interpolates smoothly and pane lease holds steady.
    app.toggle_sidebar(t0);
    assert!(app.shell_state.sidebar_collapsed);
    assert!(app.sidebar_animation.is_some());
    assert!(app.sidebar_animation_deadline().is_some());

    let t_mid = t0 + Duration::from_millis(40);
    assert!(app.tick_sidebar_animation(t_mid));
    draw_at(&mut app, &mut term, t_mid);
    let mid_width = app.effective_sidebar_width(120, t_mid);
    assert!(
        mid_width > 4 && mid_width < expanded_sidebar,
        "expected intermediate width in (4, {expanded_sidebar}), got {mid_width}"
    );
    assert_eq!(
        app.size_leases["t1"].allocated_size(),
        initial_lease,
        "pane resize must not thrash mid-animation"
    );

    // Settling at end of duration applies the collapsed width and resizes once.
    let t_end = t0 + SIDEBAR_ANIMATION_DURATION;
    assert!(app.tick_sidebar_animation(t_end));
    draw_at(&mut app, &mut term, t_end);
    assert!(app.sidebar_animation.is_none());
    assert!(app.sidebar_animation_deadline().is_none());
    assert_eq!(app.effective_sidebar_width(120, t_end), 4);
    let collapsed_lease = app.size_leases["t1"].allocated_size();
    assert!(
        collapsed_lease.0 > initial_lease.0,
        "pane should widen once settled: {collapsed_lease:?} vs {initial_lease:?}"
    );

    // Expand back: animates smoothly from 4 -> expanded_sidebar.
    let t1 = t_end + Duration::from_millis(50);
    app.toggle_sidebar(t1);
    assert!(!app.shell_state.sidebar_collapsed);
    let t1_mid = t1 + Duration::from_millis(40);
    draw_at(&mut app, &mut term, t1_mid);
    let exp_mid = app.effective_sidebar_width(120, t1_mid);
    assert!(exp_mid > 4 && exp_mid < expanded_sidebar);
    assert_eq!(app.size_leases["t1"].allocated_size(), collapsed_lease);

    let t1_end = t1 + SIDEBAR_ANIMATION_DURATION;
    draw_at(&mut app, &mut term, t1_end);
    assert!(app.sidebar_animation.is_none());
    assert_eq!(app.effective_sidebar_width(120, t1_end), expanded_sidebar);
    assert_eq!(app.size_leases["t1"].allocated_size(), initial_lease);
}

#[tokio::test]
async fn mid_animation_toggle_click_reverses_from_current_width() {
    let mut app = app_with_spaces(&[("t1", "work")]);
    let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let t0 = Instant::now();
    draw_at(&mut app, &mut term, t0);
    let expanded_sidebar = app.sidebar_width;

    app.toggle_sidebar(t0);
    let t_mid = t0 + Duration::from_millis(40);
    draw_at(&mut app, &mut term, t_mid);
    let mid_width = app.effective_sidebar_width(120, t_mid);
    assert!(mid_width > 5 && mid_width < expanded_sidebar);

    // Hit target tracks the animated sidebar's toggle button and reverses cleanly.
    let toggle_rect = app.hit_map.sidebar_toggle;
    assert_eq!(toggle_rect.x, mid_width - 2);
    app.toggle_sidebar(t_mid);
    assert!(!app.shell_state.sidebar_collapsed);
    let anim = app.sidebar_animation.expect("reversal animation");
    assert_eq!(anim.from_width, mid_width);
    assert_eq!(anim.to_width, expanded_sidebar);

    let t_done = t_mid + SIDEBAR_ANIMATION_DURATION;
    draw_at(&mut app, &mut term, t_done);
    assert!(app.sidebar_animation.is_none());
    assert_eq!(app.effective_sidebar_width(120, t_done), expanded_sidebar);
}

#[tokio::test]
async fn hidden_collapsed_mode_animates_to_zero_and_persists() {
    let root = TestDir::new("hctui-anim");
    let path = root.join("prefs.json");
    let mut config = crate::config::Config::default();
    config.ui.sidebar_collapsed_mode = SidebarCollapsedModeConfig::Hidden;
    let shell = crate::chrome::ClientShellConfig::from_config(
        &config,
        None,
        None,
        std::path::PathBuf::new(),
    )
    .with_preferences_path(path.clone());
    let (accepted, _rx) = mpsc::channel(1);
    let (rpc, sub) = crate::testcore::openers(accepted);
    let watcher = Watcher::with_openers(
        rpc,
        sub,
        hc_proto::watch::WatchParams {
            traversal: Some(crate::project::agent_traversal),
            ..hc_proto::watch::WatchParams::default()
        },
    );
    let open: OpenAttach = Arc::new(|_| unreachable!("no attach in this test"));
    let live = config.live_keybinds();
    let mut app = App::new(watcher, open, live, shell, Arc::new(Probe::default()));
    let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();

    let t0 = Instant::now();
    draw_at(&mut app, &mut term, t0);
    app.toggle_sidebar(t0);
    let saved = crate::preferences::load(&path).expect("toggle persists immediately");
    assert_eq!(saved.sidebar_collapsed, Some(true));

    let t_mid = t0 + Duration::from_millis(50);
    draw_at(&mut app, &mut term, t_mid);
    let mid = app.effective_sidebar_width(120, t_mid);
    assert!(mid > 0 && mid < app.sidebar_width);

    let t_end = t0 + SIDEBAR_ANIMATION_DURATION;
    draw_at(&mut app, &mut term, t_end);
    assert_eq!(app.effective_sidebar_width(120, t_end), 0);
    assert!(app.sidebar_animation.is_none());
}
