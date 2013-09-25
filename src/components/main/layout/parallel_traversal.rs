use std::iterator::Iterator;
use std::rt::work_queue::WorkQueue;
use std::unstable::atomics::{AtomicUint, SeqCst};
use std::unstable::sync::UnsafeAtomicRcBox;
use std::cast::transmute;
use std::vec;
use extra::dlist::MutDListIterator;
use extra::arc::Arc;
use layout::flow;

trait Compress {
    fn compress(self) -> uint;
    fn decompress(uint) -> Self;
}

pub struct ParentInfo {
    priv this_slot: *mut Option<flow::FlowContext>,
    priv parent: uint,
}

impl ParentInfo {
    fn update_child_counter (&self) -> bool {
        let mut parent = Compress::decompress::<flow::FlowContext>(self.parent);
        let ret = {
            let num_childs = parent.get_children_visited();
            num_childs.fetch_sub(1, SeqCst) == 1
        };
        let _ = parent.compress();
        ret
    }
}
/*
trait ParallelTraverse<'self> : Compress {
    fn get_children<E:Iterator<&'self Option<Self>>>(&'self self) -> E;
    fn num_children(&self) -> uint;

    fn set_parent(&mut self, parent: ParentInfo<Self>);
    fn get_parent(&self) -> Option<ParentInfo<Self>>;

    fn set_traversal(&mut self, traversal: uint);
    fn get_traversal(&self) -> uint;

    fn get_children_visited<'a>(&'a mut self) -> &'a mut AtomicUint;
    fn set_children_visited(&mut self, AtomicUint);
}*/

impl Compress for flow::FlowContext {
    fn compress(self) -> uint {
        let (tag, ptr): (uint, uint) = unsafe { transmute(self) };
        assert!(tag & !0xF == 0, "Unable to compress FlowContext");
        assert!(ptr & 0xF == 0, "Unable to compress FlowContext");

        tag | ptr
    }

    fn decompress(raw_ptr: uint) -> flow::FlowContext {
        let (tag, ptr) = (raw_ptr & 0xF, raw_ptr & !0xF);
        unsafe { transmute((tag,ptr)) }
    }
}

impl<'self> flow::FlowContext {
    fn get_children(&'self mut self) -> MutDListIterator<'self,Option<flow::FlowContext>> {
        self.mut_base().children.mut_iter()
    }
    fn num_children(&self) -> uint {
        self.base().children.len()
    }

    fn set_parent(&mut self, parent: ParentInfo) {
        do self.with_mut_base |base| {
            base.parent_info = Some(parent);
        }
    }

    fn get_parent(&self) -> Option<ParentInfo> {
        self.base().parent_info
    }

    fn set_traversal(&mut self, traversal: uint) {
        do self.with_mut_base |base| {
            base.traversal_num = traversal;
        }
    }
    fn get_traversal(&self) -> uint {
        self.base().traversal_num
    }

    fn get_children_visited<'a>(&'a mut self) -> &'a mut AtomicUint {
        &mut self.mut_base().child_counter
    }
    fn set_children_visited(&mut self, new_visited: AtomicUint) {
        do self.with_mut_base |base| {
            base.child_counter = new_visited;
        }
    }
}

struct WorkPool<T> {
    queue: UnsafeAtomicRcBox<WorkQueue<T>>,
    num_tasks: uint,
}

impl<W:Send> WorkPool<W> {
    fn new(num_tasks: uint) -> WorkPool<W> {
        unsafe {
            WorkPool {
                queue: UnsafeAtomicRcBox::new(WorkQueue::new()),
                num_tasks: num_tasks
            }
        }
    }

    /// Add an initial item of work to the queue
    fn add_work(&mut self, work: W) {
        unsafe {
            (*self.queue.get()).push(work);
        }
    }

    fn pop_work(&mut self) -> Option<W> {
        unsafe {
            (*self.queue.get()).pop()
        }
    }

    fn is_empty(&self) -> bool {
        unsafe {
            (*self.queue.get()).is_empty()
        }
    }

    /// Run the tasks on work items in the queue.
    /// The callee can obtain new work by calling the pop function,
    /// And can add work to the queue by calling the push function.
    /// Work stops when the callback returns false.
    fn execute(&mut self, 
               cb_factory: &fn() -> ~fn(push: &fn(work: W), 
                                        pop: &fn() -> Option<W>) 
                                        -> bool) {
        unsafe {
            for i in range(0,self.num_tasks) {
                let callback = cb_factory();

                let queue = self.queue.clone();
                do spawn {
                    debug!("Started slave task");
                    let queue = queue.clone().get();
                    loop {
                        // The push function simply adds work to this tasks queue.
                        // The pop function tries to pull work from the current
                        // queue, but steals from a random queue if this is not
                        // possible.
                        if !callback(|w| { (*queue).push(w) }, || {(*queue).steal()}) {
                            break;
                        }
                    }
                    debug!("Ending slave task");
                }
            }
            debug!("WorkPool::execute: finished starting tasks");
        }
    }
}


trait ParallelTraversalHelper{
    fn get_or_none(&self, uint) -> Option<TraversalType>;
}

impl ParallelTraversalHelper for 
~[(TraversalType, ~fn:Send+Freeze(&mut flow::FlowContext) -> bool)] {
    fn get_or_none(&self, index: uint) -> Option<TraversalType> {
        if index >= self.len() {
            return None;
        }

        match self[index] {
            (traversal,_) => Some(traversal)
        }
    }
}

pub enum TraversalType {
    TopDown,
    BottomUp(TraversalSubtype),
}

pub enum TraversalSubtype {
    Normal,
    Inorder
}

pub struct ParallelTraverser {
    traversals: ~[(TraversalType, ~fn:Send+Freeze(&mut flow::FlowContext) -> bool)],
}

enum EndFlag {
    NotDone,
    Done,
    Node(uint),
}

impl EndFlag {
    fn is_done(&self) -> bool {
        match *self {
            NotDone => false,
            Done => true,
            Node(_) => true,
        }
    }
}

impl ParallelTraverser {
    pub fn new(num_traversals: uint) -> ParallelTraverser {
        ParallelTraverser {
            traversals: vec::with_capacity(num_traversals + 1),
        }
    }

    pub fn add_traversal(&mut self,
            ty: TraversalType, 
            cb: ~fn:Send+Freeze(&mut flow::FlowContext) -> bool) {

        self.traversals.push((ty,cb));
    }

    pub fn run(~self, 
               mut tree: flow::FlowContext, 
               num_tasks: uint) -> flow::FlowContext {

        let mut this = self;
        if this.traversals.len() == 0 {
            return tree;
        }

        let last_node = UnsafeAtomicRcBox::new(NotDone);

        let mut work_pool = WorkPool::new(num_tasks);

        let starts_bu = match this.traversals[0] {
            (TopDown, _) => false,
            (BottomUp(_), _) => true,
        };
        // We must start with a TD so we can set the
        // parent pointers for the next traversal.
        if starts_bu {
            this.traversals.unshift((TopDown, |_| {true}));
        }
        // Initial conditions. Reset the traversal
        // counters on the starting node before
        // adding it to the work queue.
        match this.traversals[0] {
            // TD starts at the root.
            (TopDown, _) => {
                tree.set_traversal(0);
                work_pool.add_work(tree.compress());
            }

            (BottomUp(_), _) => {
                fail!("First traversal must be TopDown")
            }
        }

        

        // all threads need access to the traversals, so wrap them in an ARC
        let shared_traversals = Arc::new(this.traversals);
        unsafe {
            do work_pool.execute { 
                let this_last_node = last_node.clone();
                let this_traversals = shared_traversals.clone();
                |push, pop| {
                    let mut last_node = *this_last_node.get();
                    let traversals = this_traversals.get();
                    let node = match pop() {
                        None => {
                            None
                        }
                        Some(work) => {
                            Some(Compress::decompress::<flow::FlowContext>(work))
                        }
                    };


                    match node {
                        None => {
                            if last_node.is_done() {
                                debug!("last_node is done, ending loop");
                                false
                            } else {
                                true
                            }
                        }
                        Some(node) => {
                            match traversals[node.get_traversal()] {
                                (TopDown, ref callback) => {
                                    // visit the node then add its children
                                    // to the work queue
                                    let mut node = node;
                                    (*callback)(&mut node);

                                    let num_children = node.num_children();

                                    let cur_traversal = node.get_traversal();
                                    node.set_traversal(cur_traversal + 1);
                                    
                                    if num_children != 0 {
                                        node.set_children_visited(AtomicUint::new(num_children));

                                        // There's a bit of a dance here to get the children to 
                                        // "collectively own" the parent node:
                                        // 1) get a compressed pointer to the parent
                                        // 2) decompress the pointer from 1) to iterate over
                                        //     children
                                        // 3) copy the pointer from 1) into each child
                                        // 4) compress the pointer from 2) so we don't free the
                                        //     memory
                                        let compressed_parent = node.compress();
                                        let mut parent = Compress::decompress::<flow::FlowContext>
                                            (compressed_parent);
                                        for child in parent.get_children() {
                                            // If children are None, it means we screwed up the BU
                                            // traversal before this.
                                            child.get_mut_ref().set_traversal(cur_traversal);

                                            let child_ptr: *mut Option<flow::FlowContext> = child;
                                            child.get_mut_ref().set_parent( ParentInfo {
                                                this_slot: child_ptr,
                                                parent: compressed_parent,
                                            });

                                            push(child.take_unwrap().compress());
                                        }
                                        let _ = parent.compress();
                                    } else {
                                        // if the node is a leaf, try and start the next traversal
                                        // in the sequence
                                        match (*traversals).get_or_none(node.get_traversal()) {
                                            None => { fail!("Last traversal must be BU") }
                                            Some(BottomUp(_)) => {
                                                push(node.compress());
                                            }
                                            _ => fail!("Unsupported traversal sequence")
                                        }
                                    }
                                }

                                (BottomUp(subtype), ref callback) => {
                                    let mut node = node;
                                    match subtype {
                                        Normal => { (*callback)(&mut node); }
                                        Inorder => {
                                            if !node.is_inorder() {
                                                (*callback)(&mut node);
                                            }
                                        }
                                    }

                                    let cur_traversal = node.get_traversal();
                                    node.set_traversal(cur_traversal + 1);

                                    match node.get_parent() {
                                        // if the node is the root, try and start the next traversal
                                        // in the sequence
                                        None => {
                                            match (*traversals).get_or_none(node.get_traversal()) {
                                                None => { 
                                                    let last_ptr = this_last_node.get();
                                                    *last_ptr = Node(node.compress());
                                                    debug!("End of traversals");
                                                }
                                                Some(TopDown) => {
                                                    push(node.compress());
                                                }
                                                _ => fail!("Unsupported traversal sequence")
                                            }
                                        }

                                        Some(parent_info) => {
                                            // update_child_counter synchronizes on the parent node,
                                            // so only one thread will add the parent to the queue
                                            if parent_info.update_child_counter() {

                                                let mut parent: flow::FlowContext = Compress::decompress(parent_info.parent);
                                                parent.set_traversal(cur_traversal);
                                                // make sure we don't free the parent too soon
                                                let _ : uint = parent.compress();

                                                (*parent_info.this_slot) = (Some(node));

                                                push(parent_info.parent);
                                            }
                                        }
                                    }
                                }
                            };
                            true
                        }
                    }
                }
            };
            loop {
                debug!("Master task: checking if we need to exit");
                match *last_node.get() {
                    NotDone => {},
                    Done => fail!("Should never see Done last_node in master task"),
                    Node(node) => {
                        debug!("Ending master task");
                        return Compress::decompress::<flow::FlowContext>(node);
                    }
                };
            }
        }
    }
}

