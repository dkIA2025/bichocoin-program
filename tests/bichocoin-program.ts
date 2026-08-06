import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { assert } from "chai";
import { BichocoinProgram } from "../target/types/bichocoin_program";

describe("bichocoin-program", () => {
  // Configure the client to use the local cluster.
  anchor.setProvider(anchor.AnchorProvider.env());

  const program = anchor.workspace.bichocoinProgram as Program<BichocoinProgram>;

  it("initializes the program config", async () => {
    const [config] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("config")],
      program.programId,
    );
    const entryFee = new anchor.BN(100_000_000);

    const tx = await program.methods
      .initialize(entryFee)
      .rpc();
    console.log("Your transaction signature", tx);

    const storedConfig = await program.account.config.fetch(config);
    assert.isTrue(storedConfig.entryFee.eq(entryFee));
    assert.isTrue(storedConfig.roundCounter.eq(new anchor.BN(0)));
  });
});
