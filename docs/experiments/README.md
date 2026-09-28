# Experiments

What follows is how the contention model was arrived at on an STM32U585, in the order the questions came up. Every campaign ran from a single firmware image, since relinking the same sources moved a median by 85 cycles and would otherwise sit inside every difference being measured.

## What we expected

The starting guess was the obvious one. A competing master moves data, moving data takes bus time, so the more bandwidth it uses the more it should cost you. Under that model the fix is capacity planning: measure the aggregate traffic, leave headroom, done.

The second guess was that some memory banks are simply slower under load than others, and that the job was to find the bad bank and keep the model out of it.

Both turned out to be wrong, and the way they were wrong is what the model is built on.

## Transactions, not bytes

Two captures eleven minutes apart settled the first guess. Same transaction rate, four times the byte rate, and the cost came out at 143,929 and 143,928 cycles. The memory system is not charging you for the bytes the other master moves, it is charging you for the number of times it asks.

This matters for what you can do about it. Halving a DMA's block size while keeping its trigger rate does not help, and may well hurt, since the same number of requests now carry less payload each.

## You only pay for your own banks

The second guess looked true for a while. One bank appeared to cost the other two about five percent even when nothing was shared, and no mechanism in the reference manual explained it.

The cause was a contaminated measurement. A second memory-to-memory aggressor had been left running across a set of victim changes, which lifted the baseline by roughly fifteen thousand cycles and made the expensive column look like a property of the bank. Once the runs were repeated cleanly, the expensive column followed the stack rather than the bank, and the asymmetry disappeared.

What replaced it is simpler. A victim pays only for traffic that lands in a bank its own objects occupy. The charge is made once per bank, so putting two objects in one bank costs the same as putting one there, and moving an object around inside a bank changes nothing at all.

## Arena or stack

That raised a question the earlier work had skipped. When the numbers move as you relocate the activation arena, is it the arena driving them, or something else that happens to move at the same time.

Nine cells separate them: the arena in each of three banks crossed with the measurement thread's stack in each of three banks, with the competing traffic held fixed. The stack turned out to carry most of the cost in the read-loop victim, which is why the earlier table had looked like a statement about banks. Both objects matter, and the model adds one charge per distinct bank occupied rather than one per object.

A separate term showed up in the same work. The DMA's linked-list descriptor page is itself fetched from memory, so when it shares a bank with one of your objects you pay for the fetches as well as for the data traffic. It is about an order of magnitude smaller than the data term and it is easy to miss, since the descriptor page moves only if you move it deliberately.

## When you let ThreadX pick the address

Everything above pins the addresses by hand, which is not how a normal application is written. A ThreadX application asks `tx_byte_allocate` for a stack and takes whatever it gets, so the question is whether a placement you did not choose is even stable, and whether it can be reasoned about.

Thirty boots on the vendor path gave one address, every time, with no variation. The byte pool is first fit over a link-time buffer and nothing in that path carries state across a reset, so the determinism is by construction rather than by luck. Fifteen of those boots ran before the rest of the campaign and fifteen after, so nothing drifted in between.

The address is also predictable when you disturb it. Taking a block out of the same pool before the thread is created moves the stack by exactly the size of that block plus one allocator header, at every size tested. An unrelated allocation somewhere else in your firmware moves your stack by a known amount, and the cost follows the bank it lands in.

Two consequences. You cannot read an address off the linker map and assume it is where the object will be, since the vendor allocator hands out addresses the map does not contain. And a change that looks unrelated, adding a small buffer in another module, can relocate a hot object, which is the sort of thing that surfaces weeks later as a timing change nobody can attribute.

The same effect exists at link time. Five small static variables added for instrumentation moved the pool itself by sixteen bytes, for the same reason and with the same predictability.

## Where the model stops

Moving the stack within its bank changed the cost by eleven cycles out of 324,000 across a four kilobyte sweep, which is consistent with the model having no resolution below a bank. The sweep never crossed a bank boundary, so what a placement change across a boundary costs is measured elsewhere and not here.

The per-bank magnitudes belong to the workload, not to the part. The generated network and a bare CPU read loop give the same shape, with the read loop predicted to within one cycle in 107,974, and different numbers. So the rule can be written down once and the magnitudes have to be measured per workload, which is why a coefficient in the characterisation carries the access count it was taken at and refuses to be scaled without one.

Everything here ran on one part. Nothing in it says how the coefficients behave on another, and the borrowed-coefficient path in the audit exists precisely because that has not been tested yet.
