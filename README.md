# oasis-chain

[![CircleCI](https://circleci.com/gh/oasislabs/oasis-chain.svg?style=svg)](https://circleci.com/gh/oasislabs/oasis-chain)

A simulated Oasis blockchain for local testing.

## Build/install
```
$ git clone https://github.com/oasislabs/oasis-chain
$ cd oasis-chain
$ cargo install --path .
```
or
```
$ RUSTFLAGS='-C target-feature=+aes,+ssse3' cargo install --git https://github.com/oasislabs/oasis-chain oasis-chain
```

## Run
```
$ oasis-chain
2019-07-15 08:18:55,393 INFO  [oasis_chain] Starting Oasis local chain
Accounts (100 DEV each)
==================
(0) 0xb8b3666d8fea887d97ab54f571b8e5020c5c8b58
(1) 0xff8c7955506c8f6ae9df7efbc3a26cc9105e1797
(2) 0x0056b9346d9a64dcdd9d7be4ee3f5cf65940167d
(3) 0x4bbbf0653dab1e8abbe603fe3c4300032ff9224e
(4) 0xb99e5a84415e4bf715efd8a390344d7121015920
(5) 0xfa5c64dbcc09bdceaea11ca1f413c40031fa4412
(6) 0x17ef28e540a7cf63a8cbfd533cbbec530eac356f
(7) 0x223b7e8dda3afeb788259de0bc7bf157c8e18888
(8) 0x5e66f3176cb59205d4897509a11d117ed855502e
(9) 0x07b23940821ea777b9a26e3c8dc3027648236bbf

Private Keys
==================
(0) 0xb5144c6bda090723de712e52b92b4c758d78348ddce9aa80ca8ef51125bfb308
(1) 0x7ec6102f6a2786c03b3daf6ac4772491f33925902326a0d2d83521b964a87402
(2) 0x069f89ed3070c73586672b4d64f08dcc0f91d65dbdd201b27d5949a437035e4a
(3) 0x142b968d9b046c5545ed5d0c97c2f4b89c0ed78e19ec600d2ea8c703231d13f4
(4) 0x1a8722ce2d1f296e73a8a0de6ffecea349197188feb32e949f95f0f5d404db5d
(5) 0xf47bf050ec19b8573b32fda50436526e8c3f5b1c7f260bbdb55d4ca39585d78d
(6) 0x2424da82ad906f131674f05f207af85e7f6046fd9e0b6a4d4f37414c4933ab09
(7) 0x133e548822a035a5db2a43a091146db96f10a5c680d2114145493b921df1b19e
(8) 0xb67377abfa1a229ba56826661736ceca99d2b0be055e84498c7b0847431e4d9d
(9) 0xa08930847a93d725a62f6866afac2642eaebb4d0410610822833b0474871b7b8

HD Wallet
==================
Mnemonic:      range drive remove bleak mule satisfy mandate east lion minimum unfold ready
Base HD Path:  m/44'/60'/0'/0/{account_index}

2019-07-15 08:18:55,488 INFO  [ws] Listening for new connections on 127.0.0.1:8546.
2019-07-15 08:18:55,492 INFO  [oasis_chain] Oasis local chain is running
```

## Docker

You can also run it as a Docker container:

```
$ docker run --rm -p 127.0.0.1:8546:8546 oasislabs/oasis-chain:latest
```
