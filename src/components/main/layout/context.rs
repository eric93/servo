/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! Data needed by the layout task.

use geom::rect::Rect;
use gfx::font_context::FontContext;
use gfx::geometry::Au;
use servo_net::local_image_cache::LocalImageCache;
use servo_net::image_cache_task::ImageCacheTask;

/// Data needed by the layout task.
pub struct LayoutContext {
    image_cache: @mut LocalImageCache,
    screen_size: Rect<Au>,
}

impl LayoutContext {
    pub fn clone_for_visitor(&self, cache: ImageCacheTask) -> LayoutContext {
        LayoutContext {
            image_cache: @mut LocalImageCache(cache),
            screen_size: self.screen_size.clone(),
        }
    }
}
