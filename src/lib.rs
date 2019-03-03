#![deny(clippy::all)]
#![allow(clippy::unused_unit)]
#![feature(await_macro, futures_api, async_await, integer_atomics, gen_future)]

#[macro_use]
extern crate rental;

#[macro_use]
mod caps;
mod channel;
mod compose;
mod state;

pub use {
    crate::{
        caps::{CallsiteId, Moniker, ScopeId},
        channel::{channel, Sender},
        compose::{Compose, Scope, Scopes},
        state::{Guard, Handle},
    },
    futures::{
        future::FutureExt,
        stream::{Stream, StreamExt},
    },
    mox::component,
    std::future::Future,
};

pub(crate) mod our_prelude {
    pub use {
        futures::{
            future::{Aborted, FutureExt, FutureObj},
            task::Spawn,
        },
        log::{debug, error, info, trace, warn},
        parking_lot::Mutex,
        std::{future::Future, sync::Arc, task::Waker},
    };
}

use {
    crate::our_prelude::*,
    futures::{
        executor::ThreadPool,
        future::{AbortHandle, Abortable},
        pending,
    },
};

#[macro_export]
macro_rules! runtime {
    ($struct_name:ident: $($db_trait:path),* ) => {
        #[salsa::database( $( $db_trait ),* )]
        #[derive(Default)]
        pub struct $struct_name {
            runtime: salsa::Runtime<$struct_name>,
            scopes: $crate::Scopes,
        }

        impl salsa::Database for Toolbox {
            fn salsa_runtime(&self) -> &salsa::Runtime<Self> {
                &self.runtime
            }
        }

        impl moxie::Runtime for Toolbox {
            fn scopes(&self) -> &Scopes {
                &self.scopes
            }
        }
    };
}

pub trait Runtime: TaskBootstrapper + Send + 'static {
    fn scopes(&self) -> &Scopes;

    fn scope(&self, id: caps::ScopeId) -> Scope {
        self.scopes().get(id, self)
    }
}

// TODO make this a trait method when impl trait in trait methods works
pub async fn run<ThisRuntime, RootComponent>(
    mut runtime: ThisRuntime,
    spawner: ThreadPool,
    root_component: RootComponent,
) where
    ThisRuntime: Runtime + Unpin + 'static,
    for<'r> RootComponent: Fn(&'r ThisRuntime, Scope),
{
    let (exit_handle, exit_registration) = AbortHandle::new_pair();

    // make sure we can be woken back up and exited
    let mut waker = None;
    std::future::get_task_waker(|lw| waker = Some(lw.clone()));
    runtime.set_waker(waker.unwrap().into());
    runtime.set_top_level_exit(exit_handle);
    runtime.set_spawner(spawner);

    // this returns an error on abort, which is the only time we expect it to return at all
    let _main_compose_loop = await!(Abortable::new(
        async move {
            let root_scope = runtime.scope(caps::ScopeId::root());
            let _ensure_waker_is_set = runtime.waker();
            loop {
                root_component(&runtime, root_scope.clone());
                // unless we stash our own waker above, we'll never get woken again, be careful
                pending!();
            }
        },
        exit_registration
    ));
}

#[doc(hidden)]
#[salsa::query_group(TaskQueries)]
pub trait TaskBootstrapper: salsa::Database {
    #[salsa::input]
    fn waker(&self) -> Waker;
    #[salsa::input]
    fn spawner(&self) -> ThreadPool;
    #[salsa::input]
    fn top_level_exit(&self) -> AbortHandle;
}

#[doc(hidden)]
#[salsa::database(TaskQueries)]
#[derive(Default)]
struct TestTaskRuntime {
    runtime: salsa::Runtime<Self>,
}

#[doc(hidden)]
impl salsa::Database for TestTaskRuntime {
    fn salsa_runtime(&self) -> &salsa::Runtime<Self> {
        &self.runtime
    }
}
