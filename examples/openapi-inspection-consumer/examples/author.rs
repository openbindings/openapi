use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};
fn main() {
    let content = r#"{"document":"{\"openapi\":\"3.1.2\",\"paths\":{\"/items\":{\"get\":{\"operationId\":\"listItems\"}}}}"}"#;
    let result = pin!(openapi_inspection_consumer::inspect_and_author(
        content,
        "/paths/~1items/get"
    ))
    .poll(&mut Context::from_waker(Waker::noop()));
    match result {
        Poll::Ready(Ok(document)) => println!("{}", document.text()),
        Poll::Ready(Err(e)) => panic!("{e}"),
        Poll::Pending => panic!("use an executor for a suspending provider"),
    }
}
