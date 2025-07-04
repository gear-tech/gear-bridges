import {Entity as Entity_, Column as Column_, PrimaryColumn as PrimaryColumn_, StringColumn as StringColumn_, Index as Index_} from "@subsquid/typeorm-store"

@Entity_()
export class EthBridgeProgram {
    constructor(props?: Partial<EthBridgeProgram>) {
        Object.assign(this, props)
    }

    @PrimaryColumn_()
    id!: string

    @Index_({unique: true})
    @StringColumn_({nullable: false})
    name!: string
}
