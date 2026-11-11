# Composer panel boundaries

Eight original `gemini-3.8-flash` composer responses for `revive` and `derogatory`, four attempts each. All finished with `stop/STOP`, no safety refusal, and fewer than 8192 completion tokens.

The raw replies and feature inputs come from an archived composer investigation. Each expected document differs only by replacing `]` with `}` at panel object boundaries. Decoder tests compare the complete parsed structures. Public pipeline tests also replay the corresponding feature input and attempt: six scenes pass, while `derogatory-a1` and `derogatory-a2` retain their writing-like-mark semantic rejection.

These fixtures are synthetic learning content and contain no account credentials. They do not require a network or paid model access.
