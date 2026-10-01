//! A minimal mimic of the iced service API, so iced-style services run on guido.
//! Everything is polled by one guido task, so aborting it on owner cleanup
//! cancels the whole pipeline.

use std::any::TypeId;
use std::future::Future;

use futures::channel::mpsc;
use futures::future::BoxFuture;
use futures::stream::{BoxStream, FuturesUnordered, Stream, StreamExt};
use guido::prelude::*;

#[derive(Debug, Clone)]
#[allow(dead_code)] // Error is part of the service API; UPower never sends it
pub enum ServiceEvent<S: ReadOnlyService> {
    Init(S),
    Update(S::UpdateEvent),
    Error(S::Error),
}

pub trait ReadOnlyService: Sized {
    type UpdateEvent;
    type Error: Clone;

    fn update(&mut self, event: Self::UpdateEvent);

    fn subscribe() -> Subscription<ServiceEvent<Self>>;
}

pub trait Service: ReadOnlyService {
    type Command;

    fn command(&mut self, command: Self::Command) -> Task<ServiceEvent<Self>>;
}

/// `iced::Subscription` mimic: just a boxed event stream.
pub struct Subscription<T>(BoxStream<'static, T>);

impl<T: Send + 'static> Subscription<T> {
    /// The id is ignored: each service is subscribed exactly once.
    pub fn run_with<St>(_id: TypeId, make: impl FnOnce(&()) -> St) -> Self
    where
        St: Stream<Item = T> + Send + 'static,
    {
        Subscription(make(&()).boxed())
    }
}

/// `iced::stream::channel` mimic; the driver is polled with the receiver, not spawned.
pub fn channel<T, F, Fut>(capacity: usize, f: F) -> impl Stream<Item = T> + Send
where
    T: Send + 'static,
    F: FnOnce(mpsc::Sender<T>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (tx, rx) = mpsc::channel(capacity);
    let driver = futures::stream::once(f(tx)).map(|_| None);
    futures::stream::select(rx.map(Some), driver).filter_map(|item| async move { item })
}

/// `iced::Task` mimic.
pub struct Task<T>(TaskInner<T>);

enum TaskInner<T> {
    None,
    Perform(BoxFuture<'static, T>),
}

impl<T: Send + 'static> Task<T> {
    pub fn none() -> Self {
        Task(TaskInner::None)
    }

    pub fn perform<A: Send + 'static>(
        future: impl Future<Output = A> + Send + 'static,
        map: impl FnOnce(A) -> T + Send + 'static,
    ) -> Self {
        Task(TaskInner::Perform(Box::pin(
            async move { map(future.await) },
        )))
    }
}

/// `iced::widget::image` mimic.
pub mod image {
    use std::path::PathBuf;

    #[derive(Debug, Clone, PartialEq)]
    pub enum Handle {
        Path(PathBuf),
        Bytes(bytes::Bytes),
    }

    impl Handle {
        pub fn from_path(path: impl Into<PathBuf>) -> Self {
            Self::Path(path.into())
        }

        pub fn from_bytes(bytes: impl Into<bytes::Bytes>) -> Self {
            Self::Bytes(bytes.into())
        }
    }
}

/// `iced::widget::svg` mimic.
pub mod svg {
    use std::path::PathBuf;

    #[derive(Debug, Clone, PartialEq)]
    pub struct Handle(pub PathBuf);

    impl Handle {
        pub fn from_path(path: impl Into<PathBuf>) -> Self {
            Self(path.into())
        }
    }
}

/// `None` until `Init`. Published with `set_always`, since services have no
/// meaningful equality; consumers cut off with `create_memo`.
pub type ServiceSignal<S> = RwSignal<Option<S>>;

fn fold<S>(event: ServiceEvent<S>, service: &mut Option<S>, publish: impl Fn(S))
where
    S: ReadOnlyService + Clone,
    S::Error: std::fmt::Debug,
{
    match event {
        ServiceEvent::Init(s) => {
            *service = Some(s.clone());
            publish(s);
        }
        ServiceEvent::Update(update) => {
            if let Some(s) = service.as_mut() {
                s.update(update);
                publish(s.clone());
            }
        }
        ServiceEvent::Error(err) => {
            log::error!("service {} error: {err:?}", std::any::type_name::<S>());
        }
    }
}

pub fn run_service<S>() -> (ServiceSignal<S>, guido::prelude::Service<S::Command>)
where
    S: Service + Clone + Send + 'static,
    S::UpdateEvent: Send + 'static,
    S::Command: Send + 'static,
    S::Error: Send + std::fmt::Debug + 'static,
{
    run_service_hooked::<S>(|_| {})
}

/// [`run_service`] for services without a command channel.
pub fn run_readonly_service<S>() -> ServiceSignal<S>
where
    S: ReadOnlyService + Clone + Send + 'static,
    S::UpdateEvent: Send + 'static,
    S::Error: Send + std::fmt::Debug + 'static,
{
    run_readonly_service_hooked::<S>(|_| {})
}

pub fn run_readonly_service_hooked<S>(
    mut hook: impl FnMut(&ServiceEvent<S>) + Send + 'static,
) -> ServiceSignal<S>
where
    S: ReadOnlyService + Clone + Send + 'static,
    S::UpdateEvent: Send + 'static,
    S::Error: Send + std::fmt::Debug + 'static,
{
    let signal = create_signal(None::<S>);
    let writer = signal.writer();

    create_task(move |_ctx| async move {
        let mut events = S::subscribe().0;
        let mut service: Option<S> = None;
        let publish = move |s: S| writer.set_always(Some(s));

        while let Some(event) = events.next().await {
            hook(&event);
            fold(event, &mut service, publish);
        }
    });

    signal
}

/// The hook sees each raw event before it is folded.
pub fn run_service_hooked<S>(
    mut hook: impl FnMut(&ServiceEvent<S>) + Send + 'static,
) -> (ServiceSignal<S>, guido::prelude::Service<S::Command>)
where
    S: Service + Clone + Send + 'static,
    S::UpdateEvent: Send + 'static,
    S::Command: Send + 'static,
    S::Error: Send + std::fmt::Debug + 'static,
{
    let signal = create_signal(None::<S>);
    let writer = signal.writer();

    let svc = create_service::<S::Command, _, _>(move |mut rx, _ctx| async move {
        let mut events = S::subscribe().0;
        let mut service: Option<S> = None;
        let publish = move |s: S| writer.set_always(Some(s));
        // Driven here rather than spawned, so they end with this service.
        let mut tasks = FuturesUnordered::new();

        loop {
            let event = tokio::select! {
                ev = events.next() => match ev {
                    Some(ev) => ev,
                    None => break,
                },
                Some(ev) = tasks.next() => ev,
                cmd = rx.recv() => match cmd {
                    Some(cmd) => {
                        if let Some(s) = service.as_mut() {
                            let task = s.command(cmd);
                            // command() may mutate the service synchronously
                            publish(s.clone());
                            if let Task(TaskInner::Perform(fut)) = task {
                                tasks.push(fut);
                            }
                        }
                        continue;
                    }
                    None => break,
                },
            };

            hook(&event);
            fold(event, &mut service, publish);
        }
    });

    (signal, svc)
}
