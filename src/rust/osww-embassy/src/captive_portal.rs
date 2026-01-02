//! Captive portal helpers (host-testable).
//!
//! This module contains pure helpers that support a WiFiManager-like onboarding UX:
//! a wildcard DNS responder that points all hostnames at the provisioning AP.

/// Build a minimal DNS response for common single-question queries.
///
/// - For A/IN, returns a single A answer pointing at `ip`.
/// - For other types/classes, returns a NOERROR response with 0 answers (fast negative).
///
/// We rely on `dns-protocol` for parsing/encoding, but keep our behavior intentionally minimal:
/// - answer only A/IN queries with a single A record pointing to the captive portal IP
/// - everything else returns NOERROR with 0 answers
pub fn build_dns_wildcard_response(query: &[u8], out: &mut [u8], ip: [u8; 4]) -> Option<usize> {
    use dns_protocol::{
        Message, MessageType, Question, ResourceRecord, ResourceType, ResponseCode,
    };

    let mut questions_buf = [Question::default(); 1];
    let message = Message::read(query, &mut questions_buf, &mut [], &mut [], &mut []).ok()?;
    if message.flags().qr() != MessageType::Query {
        return None;
    }
    let question = message.questions().get(0)?;

    // Only answer A/IN.
    let is_a_in = question.ty() == ResourceType::A && question.class() == 1;

    // Build response flags: QR=1, AA=1, RD preserved, RA=0, RCODE=NOERROR.
    let mut flags = message.flags();
    flags
        .set_qr(MessageType::Reply)
        .set_authoritative(true)
        .set_recursion_available(false)
        .set_truncated(false)
        .set_response_code(ResponseCode::NoError);

    let mut resp_questions = [Question::new(
        question.name(),
        question.ty(),
        question.class(),
    )];

    let ip_bytes = ip;
    let mut answers_buf = [ResourceRecord::default(); 1];
    let answers: &mut [ResourceRecord] = if is_a_in {
        answers_buf[0] = ResourceRecord::new(question.name(), ResourceType::A, 1, 0, &ip_bytes);
        &mut answers_buf[..1]
    } else {
        &mut answers_buf[..0]
    };

    let response = Message::new(
        message.id(),
        flags,
        &mut resp_questions,
        answers,
        &mut [],
        &mut [],
    );
    response.write(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dns_protocol::{Message, MessageType, Question, ResourceRecord, ResourceType};

    fn make_query_a_example_com(id: u16) -> [u8; 29] {
        // 12-byte header + QNAME (13 bytes) + QTYPE/QCLASS (4 bytes) = 29 bytes
        let mut q = [0u8; 29];
        q[0..2].copy_from_slice(&id.to_be_bytes());
        q[2..4].copy_from_slice(&0x0100u16.to_be_bytes()); // RD=1
        q[4..6].copy_from_slice(&1u16.to_be_bytes()); // QDCOUNT=1

        // QNAME: example.com
        let mut i = 12usize;
        q[i] = 7;
        i += 1;
        q[i..i + 7].copy_from_slice(b"example");
        i += 7;
        q[i] = 3;
        i += 1;
        q[i..i + 3].copy_from_slice(b"com");
        i += 3;
        q[i] = 0;
        i += 1;

        // QTYPE=A, QCLASS=IN
        q[i..i + 2].copy_from_slice(&1u16.to_be_bytes());
        i += 2;
        q[i..i + 2].copy_from_slice(&1u16.to_be_bytes());
        q
    }

    #[test]
    fn dns_wildcard_answers_a_records() {
        let query = make_query_a_example_com(0x1234);
        let mut out = [0u8; 512];
        let len = build_dns_wildcard_response(&query, &mut out, [192, 168, 4, 1]).expect("resp");

        let mut qbuf = [Question::default(); 1];
        let mut abuf = [ResourceRecord::default(); 1];
        let msg =
            Message::read(&out[..len], &mut qbuf, &mut abuf, &mut [], &mut []).expect("parse");

        assert_eq!(msg.id(), 0x1234);
        assert_eq!(msg.flags().qr(), MessageType::Reply);
        assert!(msg.flags().authoritative());
        assert!(msg.flags().recursive()); // RD preserved
        assert_eq!(msg.questions().len(), 1);
        assert_eq!(msg.answers().len(), 1);

        let answer = msg.answers()[0];
        assert_eq!(answer.ty(), ResourceType::A);
        assert_eq!(answer.class(), 1);
        assert_eq!(answer.ttl(), 0);
        assert_eq!(answer.data(), &[192, 168, 4, 1]);
    }

    #[test]
    fn dns_wildcard_returns_no_answer_for_non_a() {
        let mut query = make_query_a_example_com(0x1);
        // Overwrite QTYPE to AAAA (28).
        let qtype_offset = query.len() - 4;
        query[qtype_offset..qtype_offset + 2].copy_from_slice(&28u16.to_be_bytes());

        let mut out = [0u8; 512];
        let len = build_dns_wildcard_response(&query, &mut out, [192, 168, 4, 1]).expect("resp");

        let mut qbuf = [Question::default(); 1];
        let mut abuf = [ResourceRecord::default(); 1];
        let msg =
            Message::read(&out[..len], &mut qbuf, &mut abuf, &mut [], &mut []).expect("parse");

        assert_eq!(msg.flags().qr(), MessageType::Reply);
        assert_eq!(msg.questions().len(), 1);
        assert_eq!(msg.answers().len(), 0);
    }
}
