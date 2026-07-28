# nivren_discord

An Edition 4 Discord foundation for bounded REST messages, command payloads, explicit retry/rate-limit decisions, secure Gateway plans, identify payloads, and typed Gateway events. Network effects remain in the application-visible `send_message` boundary; validation and scheduling decisions are pure.

The package never logs tokens and caps messages, identifiers, events, retry attempts, timeouts, and backoff. Applications still own heartbeat scheduling, resume/session storage, command registration transport, permission checks, interaction acknowledgements, global rate-limit coordination, and secret storage.
