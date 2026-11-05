// Synthetic keys and loopback TCP only. Never loads a Telegram account.
use super::*;
use grammers_session::{SessionData, storages::MemorySession};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
};

#[test]
fn connection_failure_diagnostics_distinguish_static_causes_without_error_text() {
    for (kind, label) in [
        (std::io::ErrorKind::ConnectionReset, "io_reset"),
        (std::io::ErrorKind::UnexpectedEof, "io_eof"),
        (std::io::ErrorKind::TimedOut, "io_timeout"),
    ] {
        let error = ReadError::Io(std::io::Error::new(kind, "sensitive connection context"));
        assert_eq!(read_error_kind(&error), label);
    }
    assert_eq!(read_error_kind(&ReadError::Transport(transport::Error::BadStatus { status: 404 })), "transport_status");
}

fn pool(address: std::net::SocketAddr) -> SenderPool {
    let mut data = SessionData::default();
    let dc = DcOption {
        id: data.home_dc,
        ipv4: match address {
            std::net::SocketAddr::V4(a) => a,
            _ => unreachable!(),
        },
        ipv6: SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0),
        auth_key: Some([7; 256]),
    };
    data.dc_options.insert(dc.id, dc.clone());
    let mut pool = SenderPool::new(Arc::new(MemorySession::from(data)), 1);
    pool.runner.media_options.insert(dc.id, dc);
    pool
}
fn part() -> Vec<u8> {
    tl::functions::upload::SaveBigFilePart {
        file_id: 42,
        file_part: 0,
        file_total_parts: 1,
        bytes: vec![1; 32],
    }
    .to_bytes()
}
// Decode an outgoing MTProto2 client packet with the documented x=0 keys.
// The cryptographic primitive is the existing grammers implementation.
async fn packet(stream: &mut TcpStream) -> (Vec<u8>, i64, Vec<u8>) {
    let (auth, session, _, body) = request_packet(stream).await;
    (auth, session, body)
}
async fn request_packet(stream: &mut TcpStream) -> (Vec<u8>, i64, i64, Vec<u8>) {
    let length = stream.read_u32_le().await.unwrap() as usize;
    let mut packet = vec![0; length - 4];
    stream.read_exact(&mut packet).await.unwrap();
    let encrypted = &packet[4..packet.len() - 4];
    let msg = &encrypted[8..24];
    let auth = [7; 256];
    let a = Sha256::digest([msg, &auth[..36]].concat());
    let b = Sha256::digest([&auth[40..76], msg].concat());
    let key: [u8; 32] = [&a[..8], &b[8..24], &a[24..]].concat().try_into().unwrap();
    let iv: [u8; 32] = [&b[..8], &a[8..24], &b[24..]].concat().try_into().unwrap();
    let mut plain = encrypted[24..].to_vec();
    grammers_crypto::aes::ige_decrypt(&mut plain, &key, &iv);
    let n = i32::from_le_bytes(plain[28..32].try_into().unwrap()) as usize;
    (
        encrypted[..8].to_vec(),
        i64::from_le_bytes(plain[8..16].try_into().unwrap()),
        i64::from_le_bytes(plain[16..24].try_into().unwrap()),
        plain[32..32 + n].to_vec(),
    )
}

// A synthetic encrypted rpc_result response. Reuse grammers AES and full
// transport framing; x=8 is the documented server-to-client MTProto2 direction.
async fn answer(stream: &mut TcpStream, auth_id: &[u8], session: i64, request: i64) {
    use grammers_mtproto::transport::Transport as _;
    let auth = [7; 256];
    let body = [0xf35c6d01_u32.to_le_bytes().as_slice(), &request.to_le_bytes(), &true.to_bytes()].concat();
    let message = ((std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64) << 32) | 1;
    let mut plain = [0_i64.to_le_bytes().as_slice(), &session.to_le_bytes(), &message.to_le_bytes(), &1_i32.to_le_bytes(), &(body.len() as i32).to_le_bytes(), &body].concat();
    let padding = (16 - (plain.len() + 12) % 16) % 16 + 12;
    plain.resize(plain.len() + padding, 23);
    let digest = Sha256::digest([&auth[96..128], plain.as_slice()].concat());
    let msg = &digest[8..24];
    let a = Sha256::digest([msg, &auth[8..44]].concat());
    let b = Sha256::digest([&auth[48..84], msg].concat());
    let key: [u8; 32] = [&a[..8], &b[8..24], &a[24..]].concat().try_into().unwrap();
    let iv: [u8; 32] = [&b[..8], &a[8..24], &b[24..]].concat().try_into().unwrap();
    grammers_crypto::aes::ige_encrypt(&mut plain, &key, &iv);
    let mut packet = grammers_crypto::DequeBuffer::with_capacity(plain.len() + 24, 8);
    packet.extend(auth_id.iter().copied());
    packet.extend(msg.iter().copied());
    packet.extend(plain);
    transport::Full::new().pack(&mut packet);
    stream.write_all(packet.as_ref()).await.unwrap();
}

// Stand in for an already-authorized ordinary DC connection so these tests
// exercise actual media TCPs without implementing a Telegram login server.
fn normal_control(pool: &mut SenderPool) -> mpsc::UnboundedReceiver<Rpc> {
    let (rpc_tx, rpc_rx) = mpsc::unbounded_channel();
    let abort_handle = pool.runner.connection_pool.spawn(std::future::pending());
    pool.runner.connections.push(ConnectionInfo {
        id: next_connection_id(), dc_id: pool.runner.session.home_dc_id().unwrap(), media: false,
        rpc_tx, abort_handle, uses: 0, last_reuse_log: tokio::time::Instant::now(),
    });
    rpc_rx
}

#[tokio::test]
async fn a_rejected_media_route_is_reprobed_and_restores_two_tcp_slots() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut pool = pool(listener.local_addr().unwrap());
    let dc = pool.runner.session.home_dc_id().unwrap();
    let mut control = normal_control(&mut pool);
    let budget = Arc::new(Semaphore::new(2));
    pool.handle.enable_media_pool(budget.clone());
    let handle = pool.handle.thin.clone();
    let runner = tokio::spawn(pool.runner.run());
    let h = handle.clone();
    let rejected = tokio::spawn(async move { h.raw_invoke_in_dc(dc, part()).await });
    let (mut socket, _) = listener.accept().await.unwrap();
    identity(&mut socket).await;
    socket.write_i32_le(-404).await.unwrap(); drop(socket);
    assert!(rejected.await.unwrap().is_err());
    let h = handle.clone();
    let fallback = tokio::spawn(async move { h.raw_invoke_in_dc(dc, part()).await });
    control.recv().await.unwrap().tx.send(Ok(true.to_bytes())).unwrap();
    assert_eq!(fallback.await.unwrap().unwrap(), true.to_bytes());
    tokio::time::pause(); tokio::time::advance(Duration::from_secs(11)).await; tokio::time::resume();
    let h = handle.clone();
    let probe = tokio::spawn(async move { h.raw_invoke_in_dc(dc, part()).await });
    let (mut recovered, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept()).await.unwrap().unwrap();
    let (auth_id, session, request, _) = request_packet(&mut recovered).await;
    // While probing, all other RPCs must still use the ordinary connection.
    let h = handle.clone();
    let other = tokio::spawn(async move { h.raw_invoke_in_dc(dc, part()).await });
    control.recv().await.unwrap().tx.send(Ok(true.to_bytes())).unwrap();
    assert_eq!(other.await.unwrap().unwrap(), true.to_bytes());
    assert_eq!(budget.available_permits(), 1);
    answer(&mut recovered, &auth_id, session, request).await;
    assert_eq!(tokio::time::timeout(Duration::from_secs(2), probe).await.unwrap().unwrap().unwrap(), true.to_bytes());
    let h = handle.clone();
    let third = tokio::spawn(async move { h.raw_invoke_in_dc(dc, part()).await });
    let (mut second, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept()).await.unwrap().unwrap();
    let h = handle.clone();
    let fourth = tokio::spawn(async move { h.raw_invoke_in_dc(dc, part()).await });
    assert_ne!(identity(&mut second).await.1, session);
    assert_eq!(budget.available_permits(), 0);
    third.abort(); fourth.abort();
    handle.quit(); runner.await.unwrap();
    assert_eq!(budget.available_permits(), 2);
}

#[tokio::test]
async fn canceling_a_probe_closes_only_its_socket_and_does_not_pin_a_budget() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut pool = pool(listener.local_addr().unwrap());
    let dc = pool.runner.session.home_dc_id().unwrap();
    let mut control = normal_control(&mut pool);
    let budget = Arc::new(Semaphore::new(1));
    pool.runner.media_budget = Some(budget.clone());
    pool.runner.defer_media(dc, "test_rejection");
    tokio::time::pause(); tokio::time::advance(Duration::from_secs(11)).await; tokio::time::resume();
    let handle = pool.handle.thin.clone();
    let runner = tokio::spawn(pool.runner.run());
    let h = handle.clone();
    let probe = tokio::spawn(async move { h.raw_invoke_in_dc(dc, part()).await });
    let (mut socket, _) = listener.accept().await.unwrap(); identity(&mut socket).await;
    probe.abort(); let _ = probe.await;
    let mut byte = [0];
    assert_eq!(tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte)).await.unwrap().unwrap(), 0);
    assert_eq!(budget.available_permits(), 1);
    let h = handle.clone();
    let fallback = tokio::spawn(async move { h.raw_invoke_in_dc(dc, part()).await });
    control.recv().await.unwrap().tx.send(Ok(true.to_bytes())).unwrap();
    assert!(fallback.await.unwrap().is_ok());
    handle.quit(); runner.await.unwrap();
}

#[tokio::test]
async fn probe_rpc_errors_are_preserved_and_auth_failures_never_restore_media() {
    for code in [400, 401, 403, 420, 500] {
        let mut pool = pool("127.0.0.1:1".parse().unwrap());
        let mut messages = normal_control(&mut pool);
        pool.runner.connections[0].media = true;
        let id = pool.runner.connections[0].id;
        let dc = pool.runner.connections[0].dc_id;
        let mut recovery = Recovery::new(); recovery.begin(id);
        pool.runner.media_recovery.insert(dc, recovery);
        let (tx, rx) = oneshot::channel();
        pool.runner.probe_request(0, part(), tx);
        let message = messages.recv().await.unwrap();
        message.tx.send(Err(InvocationError::Rpc(tl::types::RpcError {error_code:code, error_message:if code == 420 {"FLOOD_WAIT_3".into()} else {"TEST_REJECTED".into()}}.into()))).unwrap();
        let response = rx.await.unwrap();
        assert!(matches!(response, Err(InvocationError::Rpc(ref error)) if error.code == code));
        let event = pool.runner.connection_pool.join_next().await.unwrap().unwrap();
        pool.runner.connection_event(event);
        assert_eq!(pool.runner.media_recovery.contains_key(&dc), matches!(code, 401 | 403));
        pool.handle.quit(); pool.runner.run().await;
    }
}

#[tokio::test(start_paused = true)]
async fn an_unanswered_probe_has_a_deadline_and_reenters_cooldown() {
    let mut pool = pool("127.0.0.1:1".parse().unwrap());
    let mut messages = normal_control(&mut pool);
    pool.runner.connections[0].media = true;
    let id = pool.runner.connections[0].id;
    let dc = pool.runner.connections[0].dc_id;
    let mut recovery = Recovery::new(); recovery.begin(id);
    pool.runner.media_recovery.insert(dc, recovery);
    let (tx, rx) = oneshot::channel();
    pool.runner.probe_request(0, part(), tx);
    let _message = messages.recv().await.unwrap();
    assert!(matches!(rx.await.unwrap(), Err(InvocationError::Io(error)) if error.kind() == std::io::ErrorKind::TimedOut));
    let event = pool.runner.connection_pool.join_next().await.unwrap().unwrap();
    pool.runner.connection_event(event);
    assert!(!pool.runner.media_recovery[&dc].ready());
    assert_eq!(pool.runner.media_recovery[&dc].next_probe.saturating_duration_since(tokio::time::Instant::now()), Duration::from_secs(30));
    pool.handle.quit(); pool.runner.run().await;
}
async fn identity(stream: &mut TcpStream) -> (Vec<u8>, i64) {
    let (auth, session, body) = packet(stream).await;
    assert_eq!(
        u32::from_le_bytes(body[..4].try_into().unwrap()),
        tl::functions::InvokeWithLayer::<tl::functions::help::GetConfig>::CONSTRUCTOR_ID
    );
    assert_eq!(
        u32::from_le_bytes(body[8..12].try_into().unwrap()),
        tl::functions::InitConnection::<tl::functions::help::GetConfig>::CONSTRUCTOR_ID
    );
    assert!(
        body.ends_with(&part()),
        "media initialization must execute only its actual file query"
    );
    let query_at = body.len() - part().len();
    assert_eq!(u32::from_le_bytes(body[query_at-4..query_at].try_into().unwrap()), tl::functions::InvokeWithoutUpdates::<tl::functions::help::GetConfig>::CONSTRUCTOR_ID);
    (auth, session)
}

#[tokio::test]
async fn real_tcp_slots_share_auth_but_have_distinct_sessions_and_reconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pool = pool(listener.local_addr().unwrap());
    let dc = pool.runner.session.home_dc_id().unwrap();
    let budget = Arc::new(Semaphore::new(8));
    pool.handle.enable_media_pool(budget.clone());
    let handle = pool.handle.thin.clone();
    let runner = tokio::spawn(pool.runner.run());
    let first_handle = handle.clone();
    let first = tokio::spawn(async move { first_handle.raw_invoke_in_dc(dc, part()).await });
    let (mut one, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let first_id = identity(&mut one).await;
    let second_handle = handle.clone();
    let second = tokio::spawn(async move { second_handle.raw_invoke_in_dc(dc, part()).await });
    let (mut two, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let second_id = identity(&mut two).await;
    assert_eq!(first_id.0, second_id.0);
    assert_ne!(first_id.1, second_id.1);
    assert_eq!(budget.available_permits(), 6);
    let third_handle = handle.clone();
    let third = tokio::spawn(async move { third_handle.raw_invoke_in_dc(dc, part()).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err(),
        "more than two TCP slots"
    );
    drop(one);
    assert!(
        tokio::time::timeout(Duration::from_secs(2), first)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let replacement_handle = handle.clone();
    let replacement =
        tokio::spawn(async move { replacement_handle.raw_invoke_in_dc(dc, part()).await });
    let (mut new, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .unwrap()
        .unwrap();
    assert_ne!(identity(&mut new).await.1, first_id.1);
    second.abort();
    third.abort();
    replacement.abort();
    handle.quit();
    tokio::time::timeout(Duration::from_secs(2), runner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(budget.available_permits(), 8);
    let mut eof = [0];
    // Drain any already-written packet; the socket must close when the pool exits.
    let mut rest = vec![];
    tokio::time::timeout(Duration::from_secs(1), two.read_to_end(&mut rest))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(new.read(&mut eof).await.unwrap(), 0);
}

#[tokio::test]
async fn media_budget_is_bounded_and_idle_sockets_release_their_permits() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pool = pool(listener.local_addr().unwrap());
    let dc = pool.runner.session.home_dc_id().unwrap();
    let budget = Arc::new(Semaphore::new(1));
    pool.handle.enable_media_pool(budget.clone());
    let handle = pool.handle.thin.clone();
    let runner = tokio::spawn(pool.runner.run());
    let h = handle.clone();
    let request = tokio::spawn(async move { h.raw_invoke_in_dc(dc, part()).await });
    let (mut socket, _) = listener.accept().await.unwrap();
    identity(&mut socket).await;
    let h = handle.clone();
    let other = tokio::spawn(async move { h.raw_invoke_in_dc(dc, part()).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    request.abort();
    other.abort();
    tokio::time::sleep(Duration::from_millis(150)).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    // Let the runner observe the idle deadline and return the actual TCP lease.
    for _ in 0..20 {
        if budget.available_permits() == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(budget.available_permits(), 1);
    handle.quit();
    runner.await.unwrap();
}

#[test]
fn media_allowlist_excludes_control_and_publication() {
    assert!(is_media_request(&part()));
    assert!(!is_media_request(
        &tl::functions::help::GetConfig {}.to_bytes()
    ));
    assert!(!is_media_request(&[0; 3]));
}

#[tokio::test]
async fn media_404_returns_to_the_normal_dc_recovery_path() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pool = pool(listener.local_addr().unwrap());
    let dc = pool.runner.session.home_dc_id().unwrap();
    let budget = Arc::new(Semaphore::new(2));
    pool.handle.enable_media_pool(budget.clone());
    let handle = pool.handle.thin.clone();
    let runner = tokio::spawn(pool.runner.run());
    let h = handle.clone();
    let request = tokio::spawn(async move { h.raw_invoke_in_dc(dc, part()).await });
    let (mut media, _) = listener.accept().await.unwrap();
    identity(&mut media).await;
    media.write_i32_le(-404).await.unwrap();
    drop(media);
    assert!(
        tokio::time::timeout(Duration::from_secs(2), request)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let h = handle.clone();
    let retry = tokio::spawn(async move { h.raw_invoke_in_dc(dc, part()).await });
    let (mut control, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let (_, _, body) = packet(&mut control).await;
    assert!(
        body.ends_with(&tl::functions::help::GetConfig {}.to_bytes()),
        "404 repeated a media file on the rejected key instead of normal DC recovery"
    );
    assert_eq!(budget.available_permits(), 2);
    retry.abort();
    runner.abort();
    let _ = runner.await;
}
