use flume::{Receiver, Sender};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, define_class, msg_send, sel};
use objc2_app_kit::{NSMenu, NSMenuItem, NSStatusBar, NSVariableStatusItemLength};
use objc2_foundation::NSString;

#[derive(Clone, Copy)]
pub(crate) enum Action {
    Settings,
    Refresh,
    Quit,
}

struct TargetIvars {
    sender: Sender<Action>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "BrowsersStatusItemTarget"]
    #[ivars = TargetIvars]
    struct Target;

    impl Target {
        #[unsafe(method(statusItemAction:))]
        fn status_item_action(&self, item: &NSMenuItem) {
            let action = match item.tag() {
                0 => Action::Settings,
                1 => Action::Refresh,
                2 => Action::Quit,
                _ => return,
            };
            self.ivars().sender.send(action).ok();
        }
    }
);

impl Target {
    fn new(sender: Sender<Action>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(TargetIvars { sender });
        unsafe { msg_send![super(this), init] }
    }
}

pub(crate) struct StatusItem {
    _item: Retained<objc2_app_kit::NSStatusItem>,
    _target: Retained<Target>,
}

impl StatusItem {
    pub(crate) fn new() -> anyhow::Result<(Self, Receiver<Action>)> {
        let main_thread = MainThreadMarker::new()
            .ok_or_else(|| anyhow::anyhow!("status item must be created on the main thread"))?;
        let (sender, receiver) = flume::unbounded();
        let target = Target::new(sender);
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        let menu = NSMenu::new(main_thread);

        for (tag, title) in [(0, "Settings…"), (1, "Refresh")] {
            let menu_item = NSMenuItem::new(main_thread);
            menu_item.setTitle(&NSString::from_str(title));
            menu_item.setTag(tag);
            unsafe {
                menu_item.setTarget(Some(&target as &AnyObject));
                menu_item.setAction(Some(sel!(statusItemAction:)));
            }
            menu.addItem(&menu_item);
        }

        menu.addItem(&NSMenuItem::separatorItem(main_thread));

        let quit_item = NSMenuItem::new(main_thread);
        quit_item.setTitle(&NSString::from_str("Quit"));
        quit_item.setTag(2);
        unsafe {
            quit_item.setTarget(Some(&target as &AnyObject));
            quit_item.setAction(Some(sel!(statusItemAction:)));
        }
        menu.addItem(&quit_item);
        item.setMenu(Some(&menu));

        if let Some(button) = item.button(main_thread) {
            button.setTitle(&NSString::from_str("Browsers"));
        }

        Ok((
            Self {
                _item: item,
                _target: target,
            },
            receiver,
        ))
    }
}
