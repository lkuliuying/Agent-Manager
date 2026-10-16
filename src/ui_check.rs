use super::*;
use slint::platform::{Key, PointerEventButton, WindowEvent};

fn click(ui: &AppWindow, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}
fn key(ui: &AppWindow, text: slint::SharedString) {
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}
pub(super) fn start(ui: &AppWindow, state: Rc<RefCell<State>>) -> Timer {
    let timer = Timer::default();
    let weak = ui.as_weak();
    let mut stage = 0;
    let mut saved_y = 0.;
    let mut restored_y = 0.;
    let mut results = BTreeMap::<&str, bool>::new();
    timer.start(TimerMode::Repeated,Duration::from_millis(450),move||{
        let Some(ui)=weak.upgrade()else{return;};
        match stage {
            0=>{results.insert("initialThreeCards",state.borrow().card_models.first().is_some_and(|m|m.row_count()==3));click(&ui,400.,146.);},
            1=>{
                results.insert("nativeFilterClick",ui.get_filter()==1&&state.borrow().card_models.iter().map(|m|m.row_count()).sum::<usize>()==3);
                ui.invoke_open_detail("demo-0".into());let mut state=state.borrow_mut();
                let session=state.sessions.get_mut("demo-0").unwrap();
                for i in 0..40{session.messages.push(Message{id:format!("scroll-{i}"),author:"Agent".into(),kind:"message".into(),text:format!("第 {i} 条演示历史，用于验证消息流的阅读位置和会话切换。"),status:"completed".into(),time:"10:20".into()});}
                session.message_version+=1;state.dirty=true;
            },
            2=>{
                results.insert("detailNavigation",ui.get_view()==1&&ui.get_selected().id=="demo-0");
                ui.window().dispatch_event(WindowEvent::PointerScrolled{position:slint::LogicalPosition::new(600.,400.),delta_x:0.,delta_y:-120.});
            },
            3=>{
                let mut state=state.borrow_mut();let reading=state.readings.get(&("demo-0".into(),true));saved_y=reading.map(|r|r.y).unwrap_or(0.);
                results.insert("nativeMessageScroll",saved_y<0.&&reading.is_some_and(|r|!r.follow));
                let session=state.sessions.get_mut("demo-0").unwrap();session.messages.push(Message{id:"stream-update".into(),author:"Agent".into(),kind:"message".into(),text:"新的演示消息不会打断正在阅读的历史。".into(),status:"completed".into(),time:"10:21".into()});session.message_version+=1;state.dirty=true;
            },
            4=>{results.insert("newMessagePreservesReading",state.borrow().readings.get(&("demo-0".into(),true)).is_some_and(|r|(r.y-saved_y).abs()<1.&&!r.follow));ui.invoke_open_detail("demo-1".into());},
            5=>ui.invoke_open_detail("demo-0".into()),
            6=>{
                restored_y=state.borrow().readings.get(&("demo-0".into(),true)).map(|r|r.y).unwrap_or(0.);
                results.insert("sessionSwitchRestoresReading",(restored_y-saved_y).abs()<1.);
                for _ in 0..9{key(&ui,Key::Tab.into());}key(&ui,Key::Return.into());results.insert("keyboardNavigation",ui.get_view()==0);
                ui.set_view(0);ui.set_filter(0);ui.invoke_filter_changed(0);ui.invoke_toggle_watch("demo-2".into());
            },
            7=>{results.insert("manualCardRemoval",!state.borrow().watched.iter().any(|id|id=="demo-2"));ui.invoke_toggle_watch("demo-2".into());let mut state=state.borrow_mut();state.sessions.get_mut("demo-2").unwrap().fresh=false;state.dirty=true;},
            8=>{results.insert("manualCardAddition",state.borrow().watched.iter().any(|id|id=="demo-2"));results.insert("partialStaleState",ui.get_stale()==1&&ui.get_running()==3);ui.set_query("部署".into());ui.invoke_search("部署".into());},
            9=>{
                results.insert("titleSearch",state.borrow().sidebar.row_count()==1&&state.borrow().sidebar.row_data(0).is_some_and(|s|s.id=="demo-2"));
                ui.set_query("".into());ui.invoke_search("".into());click(&ui,70.,155.);
            },
            10=>{results.insert("sourceFilter",state.borrow().sidebar.row_count()==4&&state.borrow().sidebar.iter().all(|s|s.source_code==2));ui.set_view(2);},
            11=>{
                results.insert("sourceSetupPage",ui.get_sources().row_count()==3&&ui.get_view()==2);
                let report=serde_json::json!({"passed":results.values().all(|v|*v),"checks":results,"readingPosition":{"before":saved_y,"after":restored_y},"nativeWindow":true,"syntheticOnly":true});
                std::fs::create_dir_all("artifacts/native-ui").unwrap();std::fs::write("artifacts/native-ui/interaction-result.json",serde_json::to_vec_pretty(&report).unwrap()).unwrap();println!("{report}");slint::quit_event_loop().ok();
            },_=>{},
        }stage+=1;
    });
    timer
}
