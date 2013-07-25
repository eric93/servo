/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use servo_util::tree::TreeUtils;

pub trait TraversalMaster : TreeUtils {
    unsafe fn encode(&self) -> uint;
}

pub trait TraversalSlave {
    unsafe fn decode(uint) -> Self;
}

pub struct SequentialTraverser<Master,Slave>;

impl<M:TraversalMaster,S:TraversalSlave> SequentialTraverser<M,S> {
    pub fn new() -> SequentialTraverser<M,S> {
        return SequentialTraverser;
    }

    fn traverse_preorder(&self, tree: M, callback: &fn(S) -> bool) -> bool {
    
        unsafe {
            if !callback(TraversalSlave::decode(tree.encode())) {
                return false;
            }
        }

        for tree.each_child |kid| {
            // FIXME: Work around rust#2202. We should be able to pass the callback directly.
            if !self.traverse_preorder(kid, |a| callback(a)) {
                return false;
            }
        }

        true
    }

    fn traverse_postorder(&self, tree:M, callback: &fn(S) -> bool) -> bool {
        for tree.each_child |kid| {
            // FIXME: Work around rust#2202. We should be able to pass the callback directly.
            if !self.traverse_postorder(kid, |a| callback(a)) {
                return false;
            }
        }
        unsafe {
            callback(TraversalSlave::decode(tree.encode()))
        }
    }

    /// Like traverse_preorder, but don't end the whole traversal if the callback
    /// returns false.
    fn partially_traverse_preorder(&self, tree:M, callback: &fn(S) -> bool) {
        unsafe {
            if !callback(TraversalSlave::decode(tree.encode())) {
                return;
            }
        }

        for tree.each_child |kid| {
            // FIXME: Work around rust#2202. We should be able to pass the callback directly.
            self.partially_traverse_preorder(kid, |a| callback(a));
        }
    }

    // Like traverse_postorder, but only visits nodes not marked as inorder
    fn traverse_bu_sub_inorder(&self, tree:M, callback: &fn(S) -> bool) -> bool {
        for tree.each_child |kid| {
            // FIXME: Work around rust#2202. We should be able to pass the callback directly.
            if !self.traverse_bu_sub_inorder(kid, |a| callback(a)) {
                return false;
            }
        }

        if !self.is_inorder() {
            unsafe {
                callback(TraversalSlave::decode(tree.encode()))
            }
        } else {
            true
        }
    }
}

