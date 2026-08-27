//! iMIP (RFC 5546, pragmatic subset) — the invitation flow between mail and
//! calendar.
//!
//! Outbound ([`outbound`]): send invitations (`METHOD:REQUEST`) and RSVP
//! replies (`METHOD:REPLY`) via SMTP with an ICS attachment.
//! Inbound ([`inbound`]): process incoming ICS attachments
//! (`REQUEST`/`REPLY`/`CANCEL`) detected during IMAP sync.
//!
//! The pragmatic model (confirmed with the user): invitations travel as
//! e-mail + ICS attachment; RSVP status is tracked locally in the
//! `invitations` / `event_attendees` tables. No server-side
//! SCHEDULE-ATTENDEE / REQUEST-STATUS protocol.

pub mod inbound;
pub mod outbound;
