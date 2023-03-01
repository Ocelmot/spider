

use crate::message::UiElement;

use super::UiPage;

use std::cell::RefMut;





pub struct UiPageCursor<'a> {
    page: &'a mut UiPage,
    cursor: RefMut<'a, UiElement>,
}


impl<'a> UiPageCursor<'a> {
    pub(super) fn new(page: &'a mut UiPage, reference: RefMut<'a, UiElement>) -> Self {
        // let cursor = page.root.borrow_mut();
        // let cursor = page.get_root_ref();
        Self {
            page,
            cursor: reference,
        }
    }

}