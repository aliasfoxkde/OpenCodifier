# Choice-family gap fill — gap report (choice-v1)

Donor run `choice_full_v1` over 48000 prompts (47895 parsed, 99.8%).

Vote agreement over 15898 complete (state) sets: 3/3 unanimous = 12074, <3 parseable = 102, split (mapped queries differ) = 3266, unanimous wrong candidate = 558.

**12074 hard labels emitted of 16000 questions (75.5%)** — unanimous k-vote sets only, unanimity on the mapped query (rotation-robust); splits and unanimous wrong-candidate votes are counted, never smoothed.

Yield by true-candidate position: A = 2943/4000, B = 3227/4000, C = 2995/4000, D = 2909/4000 (the emitter placed the true query at position idx % 4, so this is the donor's position-bias readout).
