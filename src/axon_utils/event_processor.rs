use anyhow::Result;
use async_stream::stream;
use futures_core::stream::Stream;
use log::debug;
use tokio::sync::mpsc::{Sender,Receiver, channel};
use super::AxonServerHandle;
use crate::axon_server::event::{EventWithToken,GetEventsRequest};
use crate::axon_server::event::event_store_client::EventStoreClient;

#[derive(Debug)]
struct AxonEventProcessed {
    message_identifier: String,
}

pub async fn event_processor(
    axon_server_handle: AxonServerHandle
) -> Result<()> {
    let conn = axon_server_handle.conn;
    let mut client = EventStoreClient::new(conn);

    let (mut tx, rx): (Sender<AxonEventProcessed>, Receiver<AxonEventProcessed>) = channel(10);

    let outbound = create_output_stream(axon_server_handle.display_name, rx);

    debug!("Event Processor: calling open_stream");
    let response = client.list_events(outbound).await?;
    debug!("Stream response: {:?}", response);

    let mut events = response.into_inner();
    loop {
        let event_with_token = events.message().await?;
        debug!("Event with token: {:?}", event_with_token);
        if let Some(EventWithToken { event: Some(event), ..}) = event_with_token {
            tx.send(AxonEventProcessed {
                message_identifier: event.message_identifier,
            }).await?;
        }
    }
}

fn create_output_stream(client_id: String, mut rx: Receiver<AxonEventProcessed>) -> impl Stream<Item = GetEventsRequest> {
    stream! {
        debug!("Event Processor: stream: start: {:?}", rx);

        let permits_batch_size: i64 = 3;
        let mut permits = permits_batch_size * 2;

        let mut request = GetEventsRequest {
            tracking_token: 0,
            number_of_permits: permits,
            client_id: client_id,
            component_name: "Dendrite".to_string(),
            processor: "Event Processor".to_string(),
            blacklist: Vec::new(),
            force_read_from_leader: false,
        };
        yield request.clone();

        request.number_of_permits = permits_batch_size;

        while let Some(axon_event_processed) = rx.recv().await {
            debug!("Event processed: {:?}", axon_event_processed);
            permits -= 1;
            if permits <= permits_batch_size {
                debug!("Event Processor: stream: send more flow-control permits: amount: {:?}", permits_batch_size);
                yield request.clone();
                permits += permits_batch_size;
            }
            debug!("Event Processor: stream: flow-control permits: balance: {:?}", permits);
        }
    }
}