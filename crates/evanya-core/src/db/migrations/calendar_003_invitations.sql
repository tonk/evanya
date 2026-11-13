-- An invitation whose event was put in a calendar of another account keeps
-- the mail account it came in on, which answers it and receives its updates.
-- NULL when the event stays with that account.
ALTER TABLE calendar_events ADD COLUMN invite_account_id INTEGER;
CREATE INDEX idx_events_invite_account
  ON calendar_events(invite_account_id, ical_uid) WHERE invite_account_id IS NOT NULL;

-- The answers attendees mailed to our invitations (METHOD:REPLY), one per
-- message, so the reader can show each and the latest answer of every
-- attendee can be applied to the event, whatever order they arrive in.
CREATE TABLE invitation_replies (
  message_id INTEGER NOT NULL,
  account_id INTEGER NOT NULL,
  ical_uid TEXT NOT NULL,
  summary TEXT,
  attendee_email TEXT NOT NULL,
  attendee_name TEXT,
  partstat TEXT NOT NULL,
  dtstamp INTEGER,
  PRIMARY KEY (message_id, attendee_email)
);
CREATE INDEX idx_invitation_replies_uid ON invitation_replies(account_id, ical_uid);
