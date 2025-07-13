import {Entity as Entity_, Column as Column_, PrimaryColumn as PrimaryColumn_, StringColumn as StringColumn_, Index as Index_, BigIntColumn as BigIntColumn_} from "@subsquid/typeorm-store"

@Entity_()
export class GearEthBridgeMessage {
    constructor(props?: Partial<GearEthBridgeMessage>) {
        Object.assign(this, props)
    }

    @PrimaryColumn_()
    id!: string

    @Index_({unique: true})
    @StringColumn_({nullable: false})
    nonce!: string

    @BigIntColumn_({nullable: false})
    blockNumber!: bigint
}
