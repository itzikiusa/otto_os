//! Actual bundled School scene in an isolated native child. No synthetic
//! visibility events or renderer replacements; observations use its existing
//! read-only debug handle. Empty fixture classroom: no external agent/provider.
use super::{evaluate, menu, wait};
use tauri::{Emitter, Manager, Webview};

pub const INITIALIZE: &str = r#"
localStorage.setItem('otto_home_views',JSON.stringify([{id:'native-school',name:'Native school',boxes:[{id:'native-classrooms',kind:'classrooms',w:12,h:8,config:{view:'3d'}}]}]));
localStorage.removeItem('otto_home_active');localStorage.removeItem('otto_bar_spaces');localStorage.removeItem('otto_home_rotate');
"#;

fn state(child: &Webview) -> serde_json::Value {
    evaluate(child, "({hidden:document.hidden,focused:document.hasFocus(),scene:document.querySelector('.school .stage').__ottoSchool.debug()})")
}

fn layout(host: &Webview, child: &Webview, visible: bool) {
    // Exercise normal modal occlusion; a direct pane_layout call would race
    // the production host, which correctly recomputes its own visibility.
    if visible {
        evaluate(host, "Array.from(document.querySelectorAll('[role=dialog] button')).find(b=>b.textContent.trim()==='Cancel').click();true");
        wait(host, "!document.getElementById('nw-name')");
    } else {
        host.app_handle()
            .emit_to(
                tauri::EventTarget::webview("main"),
                "otto://menu",
                "new-workspace",
            )
            .unwrap();
        wait(host, "!!document.getElementById('nw-name')");
    }
    for _ in 0..100 {
        evaluate(child, "window.schoolPane=null;window.__TAURI_INTERNALS__.invoke('pane_state').then(v=>window.schoolPane=v);true");
        wait(child, "window.schoolPane!==null");
        if evaluate(child, "window.schoolPane.visible") == visible {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    println!("NATIVE_SCHOOL_OCCLUSION_DIAGNOSTIC host={} child={}",
        evaluate(host, "({url:location.href,width:innerWidth,dialogs:document.querySelectorAll('[role=dialog]').length,menus:document.querySelectorAll('.ctx-menu').length,side:document.querySelector('[data-testid=side-pane]')?.outerHTML.slice(0,1000),text:document.body.innerText.slice(0,1200)})"),
        evaluate(child, "({pane:window.schoolPane,url:location.href,hidden:document.hidden,scene:document.querySelector('.school .stage').__ottoSchool.debug()})"));
    panic!("Native modal occlusion did not set pane visible={visible}");
}

pub fn assert_native_lifecycle(host: &Webview, child: &Webview) {
    let cycles = std::env::var("OTTO_NATIVE_SCHOOL_CYCLES")
        .map(|value| {
            value
                .parse::<u32>()
                .expect("school cycles must be an integer")
        })
        .unwrap_or(3);
    assert!(
        (1..=20).contains(&cycles),
        "school cycles must be in 1..=20"
    );
    evaluate(child, "location.hash='#/home';true");
    wait(
        child,
        "(document.querySelector('.school .stage')?.__ottoSchool?.debug().frames??0)>0",
    );
    let renderer = evaluate(
        child,
        r#"(() => {const c=document.querySelector('.school canvas');const gl=c.getContext('webgl2')||c.getContext('webgl');const ext=gl.getExtension('WEBGL_debug_renderer_info');return {renderer:ext?gl.getParameter(ext.UNMASKED_RENDERER_WEBGL):gl.getParameter(gl.RENDERER),version:gl.getParameter(gl.VERSION),reducedMotion:matchMedia('(prefers-reduced-motion: reduce)').matches};})()"#,
    );
    println!("NATIVE_SCHOOL_RENDERER {renderer}");
    assert_eq!(state(child)["scene"]["missing"], serde_json::json!([]));
    // Hit the actual door through the production pointer handler. The existing
    // scene projection locates the fixture workspace, as in the browser test.
    wait(child, "document.querySelector('.school .stage').__ottoSchool.project({kind:'door',id:localStorage.getItem('otto_workspace')})?.visible===true");
    evaluate(child, "const stage=document.querySelector('.school .stage');const door=stage.__ottoSchool.project({kind:'door',id:localStorage.getItem('otto_workspace')});const target=stage.querySelector('canvas');for(const type of ['pointerdown','pointerup'])target.dispatchEvent(new PointerEvent(type,{bubbles:true,clientX:door.x,clientY:door.y+60,pointerId:1,pointerType:'mouse',button:0}));true");
    wait(
        child,
        "document.querySelector('.school .stage').__ottoSchool.debug().view.kind==='room'",
    );
    evaluate(
        child,
        "window.nativeSchool=document.querySelector('.school .stage').__ottoSchool;true",
    );
    let room = state(child)["scene"]["view"].clone();
    for cycle in 1..=cycles {
        layout(host, child, false);
        std::thread::sleep(std::time::Duration::from_millis(200));
        let hidden = state(child);
        std::thread::sleep(std::time::Duration::from_millis(600));
        let later = state(child);
        assert_eq!(
            hidden["scene"]["frames"], later["scene"]["frames"],
            "hidden native School must suspend render work"
        );
        assert_eq!(later["scene"]["view"], room);
        assert_eq!(hidden["scene"]["camera"], later["scene"]["camera"]);
        layout(host, child, true);
        let frames = later["scene"]["frames"].as_u64().unwrap();
        wait(
            child,
            &format!(
                "document.querySelector('.school .stage').__ottoSchool.debug().frames>{frames}"
            ),
        );
        assert_eq!(state(child)["scene"]["view"], room);
        assert_eq!(
            evaluate(
                child,
                "window.nativeSchool===document.querySelector('.school .stage').__ottoSchool"
            ),
            true
        );
        println!(
            "NATIVE_SCHOOL_CYCLE {cycle} hidden={hidden} restored={}",
            state(child)
        );
    }
    menu(host, "Detach side pane");
    wait(
        host,
        "!document.querySelector('[data-testid=split-divider]')",
    );
    // Keep the detached fixture physically exposed when the host takes focus:
    // overlapping windows can correctly mark the child document hidden.
    let host_window = host.window();
    let original_position = host_window.outer_position().unwrap();
    let original_size = host_window.inner_size().unwrap();
    let monitor = host_window.current_monitor().unwrap().unwrap();
    let area = monitor.work_area();
    let origin = area.position.to_logical::<f64>(monitor.scale_factor());
    let size = area.size.to_logical::<f64>(monitor.scale_factor());
    let width = (size.width / 2.0 - 24.0).max(400.0);
    let height = size.height.min(720.0);
    host_window
        .set_size(tauri::LogicalSize::new(width, height))
        .unwrap();
    host_window.set_position(origin).unwrap();
    let detached = host
        .app_handle()
        .get_window("otto-pane-window-main")
        .unwrap();
    detached
        .set_size(tauri::LogicalSize::new(width, height))
        .unwrap();
    detached
        .set_position(tauri::LogicalPosition::new(
            origin.x + width + 24.0,
            origin.y,
        ))
        .unwrap();
    host.window().set_focus().unwrap();
    host.set_focus().unwrap();
    wait(child, "!document.hasFocus()&&!document.hidden");
    std::thread::sleep(std::time::Duration::from_millis(200));
    let before = state(child);
    std::thread::sleep(std::time::Duration::from_millis(600));
    let after = state(child);
    for sample in [&before, &after] {
        assert_eq!(sample["hidden"], false);
        assert_eq!(sample["focused"], false);
    }
    assert!(
        after["scene"]["frames"].as_u64().unwrap()
            > before["scene"]["frames"].as_u64().unwrap() + 1,
        "visible unfocused native School must keep rendering after focus settles"
    );
    assert_eq!(state(child)["scene"]["view"], room);
    println!("NATIVE_SCHOOL_VISIBLE_UNFOCUSED {}", state(child));
    menu(child, "Return to split");
    wait(
        host,
        "!!document.querySelector('[data-testid=split-divider]')",
    );
    host_window.set_size(original_size).unwrap();
    host_window.set_position(original_position).unwrap();
    assert_eq!(
        evaluate(
            child,
            "window.nativeSchool===document.querySelector('.school .stage').__ottoSchool"
        ),
        true
    );
    assert_eq!(state(child)["scene"]["view"], room);
    println!("PASS native School: real WebGL scene/assets, room transition, {cycles} native hidden/resume cycles, zero hidden frames, visible unfocused detached rendering, same scene and room after return");
}
