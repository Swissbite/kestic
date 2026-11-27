// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! A slow stand-in for the daemon: it owns its own bus name, forwards every method call it
//! receives to the real daemon and passes the reply (or the error) on only after a delay. The
//! real daemon's signals reach a GUI directly, since the GUI matches them by interface and path.

use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;
use zbus::Structure;
use zbus::export::ordered_stream::OrderedStreamExt;
use zbus::message::Type;

/// Handle of a running stand-in; it serves until the test process ends.
pub struct DelayedDaemon {
    pub name: String,
}

impl DelayedDaemon {
    /// Takes `front` on the bus at `address` and forwards each call to `back` after `delay`.
    pub fn start(address: &str, front: &str, back: &str, delay: Duration) -> Self {
        Self::start_with(address, front, back, move |_, _| delay)
    }

    /// Like [`Self::start`], with the delay of each call chosen by its method name and its first
    /// text argument (empty when it has none).
    pub fn start_with(
        address: &str,
        front: &str,
        back: &str,
        delay_of: impl Fn(&str, &str) -> Duration + Send + Sync + 'static,
    ) -> Self {
        let delay_of = Arc::new(delay_of);
        let (ready_tx, ready_rx) = mpsc::channel();
        let (address, front_name, back) =
            (address.to_string(), front.to_string(), back.to_string());
        thread::spawn(move || {
            zbus::block_on(async move {
                let conn = zbus::connection::Builder::address(address.as_str())
                    .name(front_name.as_str())
                    .build()
                    .await
                    .expect("front connection");
                let back_conn = zbus::connection::Builder::address(address.as_str())
                    .build()
                    .await
                    .expect("back connection");
                let mut calls = zbus::MessageStream::from(&conn);
                ready_tx.send(()).expect("report ready");
                while let Some(Ok(call)) = calls.next().await {
                    if call.message_type() != Type::MethodCall {
                        continue;
                    }
                    let (conn, back_conn, back) = (conn.clone(), back_conn.clone(), back.clone());
                    let delay = delay_of(
                        call.header().member().map_or("", |m| m.as_str()),
                        &first_text(&call),
                    );
                    thread::spawn(move || {
                        thread::sleep(delay);
                        zbus::block_on(forward(&conn, &back_conn, &back, &call));
                    });
                }
            });
        });
        ready_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("delayed daemon ready");
        Self {
            name: front.to_string(),
        }
    }
}

/// The first argument of `call` when it is a text, else empty.
fn first_text(call: &zbus::Message) -> String {
    let body = call.body();
    if !body.signature().to_string().starts_with('s') {
        return String::new();
    }
    body.deserialize::<Structure<'_>>()
        .ok()
        .and_then(|args| {
            args.fields().first().and_then(|first| match first {
                zbus::Variant::Str(text) => Some(text.to_string()),
                _ => None,
            })
        })
        .unwrap_or_default()
}

/// Sends `call` on to `back` and answers it with what `back` replied.
async fn forward(
    conn: &zbus::Connection,
    back_conn: &zbus::Connection,
    back: &str,
    call: &zbus::Message,
) {
    let header = call.header();
    let (Some(path), Some(member)) = (header.path(), header.member()) else {
        return;
    };
    let body = call.body();
    let reply = if body.signature().to_string().is_empty() {
        back_conn
            .call_method(Some(back), path, header.interface(), member, &())
            .await
    } else {
        let args: Structure<'_> = body.deserialize().expect("call body");
        back_conn
            .call_method(Some(back), path, header.interface(), member, &args)
            .await
    };
    let sent = match reply {
        Ok(reply) => {
            let reply_body = reply.body();
            if reply_body.signature().to_string().is_empty() {
                conn.reply(&header, &()).await
            } else {
                let values: Structure<'_> = reply_body.deserialize().expect("reply body");
                conn.reply(&header, &values).await
            }
        }
        Err(zbus::Error::MethodError(name, text, _)) => {
            conn.reply_error(&header, name, &text.unwrap_or_default())
                .await
        }
        Err(err) => {
            conn.reply_error(
                &header,
                "org.freedesktop.DBus.Error.Failed",
                &err.to_string(),
            )
            .await
        }
    };
    sent.expect("answer the forwarded call");
}
