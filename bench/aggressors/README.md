# Aggressors

All aggressors implement one interface, so the sweep driver treats them uniformly and Wi-Fi can be added or dropped without touching the driver.

`cpu_hog` is the control. It creates contention that is not memory contention, which is what lets the analysis separate scheduling effects from bus effects. It is not itself evidence of anything.

`gpdma_m2m` is the scientific aggressor: memory to memory, swept bandwidth and burst size, fully controlled.

`audio_p2m` is the realistic periodic one, 16 kHz microphone capture through circular DMA.

`wifi_spi` is optional and never load bearing. Live traffic is less controlled than a swept DMA aggressor, so it belongs in the demonstration.
