use byte_unit::{Byte, ByteUnit};
use bytes::{Bytes, BytesMut};
use futures::future::err;
use futures::stream::{Stream, TryStream, TryStreamExt};
use futures::{SinkExt, StreamExt};
use s2n_quic::client::Connect;
use s2n_quic::{Client, Server};
use std::error::Error;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufStream};
use tokio_util::codec::{Framed, FramedRead, LengthDelimitedCodec};

/// NOTE: this certificate is to be used for demonstration purposes only!
pub static CERT_PEM: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../cert.pem"));
/// NOTE: this certificate is to be used for demonstration purposes only!
pub static KEY_PEM: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../key.pem"));

const MAX_DATAGRAM_SIZE: Byte = Byte::from_bytes(1350);

#[derive(PartialOrd, PartialEq)]
pub enum VerifyPeer {
    Yes,
    No,
}

pub struct QuicConfig {
    verify_peer: VerifyPeer,
    max_idle_timeout: Duration,
    max_recv_udp_payload_size: Byte,
    max_send_udp_payload_size: Byte,
}

impl Default for QuicConfig {
    fn default() -> Self {
        Self {
            verify_peer: VerifyPeer::No,
            max_idle_timeout: Duration::from_secs(10),
            max_recv_udp_payload_size: MAX_DATAGRAM_SIZE,
            max_send_udp_payload_size: MAX_DATAGRAM_SIZE,
        }
    }
}

pub async fn server() -> Result<(), Box<dyn Error>> {
    let mut server =
        Server::builder().with_tls((CERT_PEM, KEY_PEM))?.with_io("127.0.0.1:4433")?.start()?;

    while let Some(mut connection) = server.accept().await {
        // spawn a new task for the connection
        tokio::spawn(async move {
            eprintln!("Connection accepted from {:?}", connection.remote_addr());
            while let Ok(Some(stream)) = connection.accept_bidirectional_stream().await {
                // spawn a new task for the stream
                tokio::spawn(async move {
                    let remote_addr = stream.connection().remote_addr();
                    eprintln!("Stream opened from {:?}", remote_addr);
                    let stream = BufStream::new(stream);
                    let codec = LengthDelimitedCodec::builder()
                        .length_field_type::<u64>()
                        .new_framed(stream);
                    let mut codec_stream = codec.into_stream();
                    while let Some(Ok(data)) = codec_stream.next().await {
                        println!("data received: {:?}", data);
                        codec_stream.send(data.freeze()).await.expect("stream should be open");
                    }

                    // echo any data back to the stream
                    // while let Ok(Some(data)) = stream.receive().await {
                    //     stream.send(data).await.expect("stream should be open");
                    // }

                    eprintln!("stream task done for {:?}", remote_addr);
                });
            }

            eprintln!("Connection finished for {:?}", connection.remote_addr());
        });
    }

    Ok(())

    // let sock = UdpSocket::bind("0.0.0.0:8080").await?;
    // let r = Arc::new(sock);
    // let s = r.clone();
    // let (tx, mut rx) = mpsc::channel::<(Vec<u8>, SocketAddr)>(1_000);
    //
    // tokio::spawn(async move {
    //     while let Some((bytes, addr)) = rx.recv().await {
    //         let len = s.send_to(&bytes, &addr).await.unwrap();
    //         println!("{:?} bytes sent", len);
    //     }
    // });
    //
    // let mut buf = vec![0; 65535];
    // loop {
    //     tokio::select! {
    //         Ok(_) = r.readable() => {
    //             'read: loop {
    //                 match r.try_recv_from(&mut buf) {
    //                     Ok((len, from)) => {
    //                         println!("recv UDP {} bytes", len);
    //                         let pkt_buf = &mut buf[..len];
    //                         let hdr = match quiche::Header::from_slice(pkt_buf, quiche::MAX_CONN_ID_LEN) {
    //                             Ok(v) => v,
    //                             Err(e) => {
    //                                 println!("Parsing packet header failed: {:?}", e);
    //                                 continue 'read;
    //                             }
    //                         };
    //                         println!("got packet: {:?}", hdr);
    //
    //                     },
    //                     Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
    //                         break 'read;
    //                     },
    //                     Err(e) => {
    //                         println!("try_recv_from() failed: {:}", e);
    //                     }
    //                 }
    //             }
    //         }
    //     }

    // let (len, addr) = r.recv_from(&mut buffer).await?;
    // println!("{:} bytes received from {:?}", len, addr);
    // tx.send((buffer[..len].to_vec(), addr)).await.unwrap();
    // }
}

pub async fn client() -> Result<(), Box<dyn Error>> {
    let client = Client::builder().with_tls(CERT_PEM)?.with_io("0.0.0.0:0")?.start()?;

    let addr: SocketAddr = "127.0.0.1:4433".parse()?;
    let connect = Connect::new(addr).with_server_name("localhost");
    let mut connection = client.connect(connect).await?;

    // ensure the connection doesn't time out with inactivity
    connection.keep_alive(true)?;

    // open a new stream and split the receiving and sending sides
    let stream = connection.open_bidirectional_stream().await?;
    let stream = BufStream::new(stream);

    let codec = LengthDelimitedCodec::builder().length_field_type::<u64>().new_framed(stream);
    let mut codec_stream = codec.into_stream();
    let msg = vec![97; 2_000];
    codec_stream.send(Bytes::from(msg)).await?;

    // let (r, s) = codec_stream.split();
    // let (mut receive_stream, mut send_stream) = stream.split();

    // spawn a task that copies responses from the server to stdout
    tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        loop {
            println!("tick");
            let resp = codec_stream.try_next().await;
            match resp {
                Ok(data) => match data {
                    None => {
                        println!("no data");
                    }
                    Some(data2) => {
                        println!("data2: {:?}", data2);
                    }
                },
                Err(err) => {
                    println!("error: {:?}", err)
                }
            }
        }
    })
    .await?;
    //
    // // copy data from stdin and send it to the server
    // let stdin = tokio_stdin_stdout::stdin(0);
    // let mut stdin = BufReader::new(stdin);
    // for Ok(line) in stdin.lines() {
    //     codec_stream.send(line.into()).await?;
    // }

    // tokio::io::copy(&mut stdin, &mut codec_stream.into_stream()).await?;

    Ok(())

    // let quic_config = QuicConfig::default();
    // let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION).unwrap();
    // config.verify_peer(quic_config.verify_peer == VerifyPeer::Yes);
    // config.set_max_idle_timeout(quic_config.max_idle_timeout.as_millis() as u64);
    // config
    //     .set_max_send_udp_payload_size(quic_config.max_send_udp_payload_size.get_bytes() as usize);
    // config
    //     .set_max_recv_udp_payload_size(quic_config.max_recv_udp_payload_size.get_bytes() as usize);
    // let mut out = vec![0; quic_config.max_send_udp_payload_size.get_bytes() as usize];
    //
    // // Generate a random source connection ID for the connection.
    // let mut scid = vec![0; quiche::MAX_CONN_ID_LEN];
    // let rng = SystemRandom::new();
    // rng.fill(&mut scid[..]).unwrap();
    // let scid = quiche::ConnectionId::from_ref(&scid);
    //
    // let local_addr = "0.0.0.0:8081".parse().unwrap();
    // let mut s = UdpSocket::bind(local_addr).await?;
    //
    // let peer_addr = "127.0.0.1:8080".parse().unwrap();
    // let mut conn = quiche::connect(None, &scid, peer_addr, &mut config).unwrap();
    // println!("connecting to {:} with scid {:?}", peer_addr, scid,);
    //
    // let (write, send_info) = conn.send(&mut out).expect("initial send failed");
    //
    // while let Err(e) = s.send_to(&out[..write], send_info.to).await {
    //     if e.kind() == std::io::ErrorKind::WouldBlock {
    //         println!("{} -> {}: send() would block", s.local_addr().unwrap(), send_info.to);
    //         continue;
    //     }
    //
    //     println!("send() failed: {:?}", e);
    //     return Err(Error);
    //     // return Err(ClientError::Other(format!("send() failed: {:?}", e)));
    // }
    //
    // println!("written {}", write);

    // sock.connect("127.0.0.1:8080").await?;
    // let len = sock.send(b"foo").await?;
    // println!("{:?} bytes send", len);
    // Ok(())
}

#[cfg(test)]
mod tests {
    use std::assert_eq;

    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
