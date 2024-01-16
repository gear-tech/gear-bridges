// SPDX-License-Identifier: GPL-3.0
/*
    Copyright 2021 0KIMS association.

    This file is generated with [snarkJS](https://github.com/iden3/snarkjs).

    snarkJS is a free software: you can redistribute it and/or modify it
    under the terms of the GNU General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    snarkJS is distributed in the hope that it will be useful, but WITHOUT
    ANY WARRANTY; without even the implied warranty of MERCHANTABILITY
    or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public
    License for more details.

    You should have received a copy of the GNU General Public License
    along with snarkJS. If not, see <https://www.gnu.org/licenses/>.
*/

pragma solidity >=0.7.0 <0.9.0;

contract Groth16Verifier {
    // Scalar field size
    uint256 constant r    = 21888242871839275222246405745257275088548364400416034343698204186575808495617;
    // Base field size
    uint256 constant q   = 21888242871839275222246405745257275088696311157297823662689037894645226208583;

    // Verification Key data
    uint256 constant alphax  = 20491192805390485299153009773594534940189261866228447918068658471970481763042;
    uint256 constant alphay  = 9383485363053290200918347156157836566562967994039712273449902621266178545958;
    uint256 constant betax1  = 4252822878758300859123897981450591353533073413197771768651442665752259397132;
    uint256 constant betax2  = 6375614351688725206403948262868962793625744043794305715222011528459656738731;
    uint256 constant betay1  = 21847035105528745403288232691147584728191162732299865338377159692350059136679;
    uint256 constant betay2  = 10505242626370262277552901082094356697409835680220590971873171140371331206856;
    uint256 constant gammax1 = 11559732032986387107991004021392285783925812861821192530917403151452391805634;
    uint256 constant gammax2 = 10857046999023057135944570762232829481370756359578518086990519993285655852781;
    uint256 constant gammay1 = 4082367875863433681332203403145435568316851327593401208105741076214120093531;
    uint256 constant gammay2 = 8495653923123431417604973247489272438418190587263600148770280649306958101930;
    uint256 constant deltax1 = 11559732032986387107991004021392285783925812861821192530917403151452391805634;
    uint256 constant deltax2 = 10857046999023057135944570762232829481370756359578518086990519993285655852781;
    uint256 constant deltay1 = 4082367875863433681332203403145435568316851327593401208105741076214120093531;
    uint256 constant deltay2 = 8495653923123431417604973247489272438418190587263600148770280649306958101930;

    
    uint256 constant IC0x = 12696637186282207704124679416512719546483426316914488802845495546409727984352;
    uint256 constant IC0y = 5551975048707908594815373844650534560363154379572251188426726434992483679891;
    
    uint256 constant IC1x = 9445463247312929625328796210350318534806360645196585435917799168013718009326;
    uint256 constant IC1y = 15075990677086291094798944959409172832962509913952651125092773204457168428351;
    
    uint256 constant IC2x = 12793468507636429934891882153157688759877859608217252986973198812611722008047;
    uint256 constant IC2y = 5344477415930242245042500640585264492527523396446079312101569813846343776445;
    
    uint256 constant IC3x = 1554355556707526128799204545981642362425559800773513081321324578171197406813;
    uint256 constant IC3y = 9705726198222288505513145452004971489047140167851143203993189595289845048776;
    
    uint256 constant IC4x = 6224255169820209205498767264034287912187495069470803774745642501485773675521;
    uint256 constant IC4y = 8820897264947220692879266199016080191362863952628318370881695687680591739048;
    
    uint256 constant IC5x = 10754561295420639877384917537499348744649574949328314928181915952651631588778;
    uint256 constant IC5y = 18824011623518220453591921628906679906225026098741336987674071756761901678966;
    
    uint256 constant IC6x = 15650591183504726679400849291709948983779190689612863494097390249574869078638;
    uint256 constant IC6y = 5235777594152286188589246999258170199229994045898713499895099300985398384837;
    
    uint256 constant IC7x = 19643353121371946053730860789663253561243312400123703238287567695581826358769;
    uint256 constant IC7y = 4093861866989225996041761499047939857429840231142403797928265860564680774910;
    
    uint256 constant IC8x = 2108504981172997734485442642969674126467692794902639019336482660870221336795;
    uint256 constant IC8y = 386423546350984152699276847479398496714237801261718571915553631808337971399;
    
    uint256 constant IC9x = 14139463562739082151632244288786440322994579313218676958217314531937346815023;
    uint256 constant IC9y = 10368127790845776632830252931806774533558729800974155001893328528025665240903;
    
    uint256 constant IC10x = 13228830461967225161887530475918227109392434963339251927905934224539196525306;
    uint256 constant IC10y = 19440704150483238001749252107744403825305612460645226630492112215415228150277;
    
    uint256 constant IC11x = 18224934267139692316585978111981511568077619619411330829829515935545546742437;
    uint256 constant IC11y = 3903588519933772576987805034218058234629363077220514804177666009150036267523;
    
    uint256 constant IC12x = 10900615489287283158873254281031410859416958850203105832667488924993760577546;
    uint256 constant IC12y = 6453440558154369906972377978946707488652483964698078052013575693186323543594;
    
    uint256 constant IC13x = 3495685844800301809073289615461397083718449658089341756635772289735458024757;
    uint256 constant IC13y = 6413291347591948435042187050414551979878136498880810715158291115381488696426;
    
    uint256 constant IC14x = 5974910985833306570367629157349492837905676143415016145794453409144429150909;
    uint256 constant IC14y = 5823399031976173875038007892525663717567909240574905374979667227133106046614;
    
    uint256 constant IC15x = 1053029471857846020716810655802240276615787210475630981664551914359674506884;
    uint256 constant IC15y = 19558472111681332757611342459383967077209637543831807283463912845930270468628;
    
    uint256 constant IC16x = 19566521545254831747605996487356224711512955853227664658080153091014676067984;
    uint256 constant IC16y = 12705127007338483150429755273449926106251634408337628086062868339139993459455;
    
    uint256 constant IC17x = 4666371447719436673313394423020008471682972833764973561271389630517306358157;
    uint256 constant IC17y = 10468557545869745976208377288422698959686946476363620613377894337448453559772;
    
    uint256 constant IC18x = 8955654374698484013218257783933588827710129902543128099834596881531156009826;
    uint256 constant IC18y = 12342104253445663548295918888460251688411914049368098561973783571996514345283;
    
    uint256 constant IC19x = 7948998979631156810174632667337720790285958907858936362338250922590414286204;
    uint256 constant IC19y = 19341478669213475655909233635725630036233761278247135658708188400923760025726;
    
    uint256 constant IC20x = 4165407587234967036025091663850908147589770817805039623046565660882160547879;
    uint256 constant IC20y = 16662854532064537909150805817550087577219303747407711515405536570182046156346;
    
    uint256 constant IC21x = 10094759183272157876506991970881240296040357620219095700766845444767422261982;
    uint256 constant IC21y = 413272978251843998816764060700656489043400038956771228865454831810037220677;
    
    uint256 constant IC22x = 11319617780722235651683538908515458120859136722089651647836579478427878636962;
    uint256 constant IC22y = 10177081886199877360454627623728880837348141169262411983050010813669351667278;
    
    uint256 constant IC23x = 19901065266701236458741714661337013289317049562256096374471846929262685439662;
    uint256 constant IC23y = 11514882114939237954739799040525200219939630050341404722869564558897211143137;
    
    uint256 constant IC24x = 17238118596072089258012499106435872339481123545270946483735689366744430441263;
    uint256 constant IC24y = 709335796199958165246201963594260882597404019174724937096299419658703600649;
    
    uint256 constant IC25x = 11887094852093138210712385408198927194327319175605158947954525138895497735763;
    uint256 constant IC25y = 10166692684786506161739485974007110419605859266950844283616823143613926328553;
    
    uint256 constant IC26x = 10230079113611864398660979874015936102714194607324645818194604085108525912628;
    uint256 constant IC26y = 16402621290627681369019723010152924082317241552475306522236588638145103847688;
    
    uint256 constant IC27x = 21879427384910947514850443187954390269721932258860080037922231821443096797431;
    uint256 constant IC27y = 11978567063357328090646297254826156212601746553653147674880580100765772357984;
    
    uint256 constant IC28x = 19130221369602496491172225730161889411594207633752746131142549703070138296827;
    uint256 constant IC28y = 11439415592229768582890370794146985212930298286844416334795858643333806395761;
    
    uint256 constant IC29x = 2657777034951870120924937945589034393319055302247198340951146986906865824079;
    uint256 constant IC29y = 4222051717530774578003643947661350502988758811701683127862631351778565713920;
    
    uint256 constant IC30x = 8282483376461407094625081505422493879729179426999627003664530312889564574830;
    uint256 constant IC30y = 5829604589143263141721978849423944821308050937639634012617188153051695176414;
    
    uint256 constant IC31x = 2154630716957230751273884398020848015344208503877532386684080479365280346937;
    uint256 constant IC31y = 14106132611024026968051906199808730709991046918751578654231569155873827279803;
    
    uint256 constant IC32x = 3894570382088594060355415369158650748984049000059975514701824047430394439017;
    uint256 constant IC32y = 11876311106903998584190904719469851599162958335389001388544379455225956341656;
    
    uint256 constant IC33x = 54908881664696710492798541752396543891646418010229772217600141355785377365;
    uint256 constant IC33y = 12269294836452622816834109508048877253685911126152957736457367860955281884106;
    
    uint256 constant IC34x = 11499372803343335711893243982818711386350392933112106694439163103092325873302;
    uint256 constant IC34y = 20952176528580412639139075659508944634105448158689683491668068496896663436591;
    
    uint256 constant IC35x = 279854409263721441796348180216838644521811727704391261742522340365489087740;
    uint256 constant IC35y = 10974915755921330099588067586747215828657609328841133103395159368245907206677;
    
    uint256 constant IC36x = 11193973864356703326470649527163425394059329179915782108123077548709833200194;
    uint256 constant IC36y = 11931514029330400154837849272159548165840881551288889544127700706336622256532;
    
    uint256 constant IC37x = 21414700037763521884346892077057365316872197750972216002228424185177579952731;
    uint256 constant IC37y = 18444160635069130267176911770591464488799160301057093765601653593922463770230;
    
    uint256 constant IC38x = 4488000743430702474405810491329795105668276155174731519787999900816844173438;
    uint256 constant IC38y = 15882587622484099356948925285501131492468015821656281344349616469845654548599;
    
    uint256 constant IC39x = 3292718502450223596434870962737059687033706464767363335438017093107775254035;
    uint256 constant IC39y = 14440392091761992903000422570679945812160184059105062556072961572365924060036;
    
    uint256 constant IC40x = 3478409265355538471746512256932259742583165603663825671416303869193896307804;
    uint256 constant IC40y = 2543717912759553482506411778214769116943167295582307133304276872901099719165;
    
    uint256 constant IC41x = 11119972867496735602010063661469057173015937200657037734383669548814972222744;
    uint256 constant IC41y = 1403496632201907567814443515737016373152600103836307094983145778558047055996;
    
    uint256 constant IC42x = 1594715869684536901865914333265138214515111178824700531828735205411580303495;
    uint256 constant IC42y = 11915490067229294321374608166446651645179999887073829925961448918463541736070;
    
    uint256 constant IC43x = 33293020974038937530123977781747993901326183583943779480302277748681540830;
    uint256 constant IC43y = 10961372827649579841572250451401615701553811478691007329393203664781188969507;
    
    uint256 constant IC44x = 17800548141278349839136513980527770285802386208311789070321342705174022356768;
    uint256 constant IC44y = 4413355602694597997880879312307004486719501566821305612258960698115635219362;
    
    uint256 constant IC45x = 13831775088134828239890170416131828867014542628276511653055697172105466851494;
    uint256 constant IC45y = 8126407279667994670754030228137139939045883775256494574908704007503249179368;
    
    uint256 constant IC46x = 19231227392288908100508706498059453897087646756075573296305605357649292334541;
    uint256 constant IC46y = 9172385885239714580151893851217665052905228047074187081683650376274494399874;
    
    uint256 constant IC47x = 4046453073259254836096276989975358031657964032197726817857366148803371966521;
    uint256 constant IC47y = 10557223215421304662021203902644285382156589688240661533822542386500640221586;
    
    uint256 constant IC48x = 14197907621183948217050140903759774334007725798909568527562497142480795629085;
    uint256 constant IC48y = 10781276875270904081178943179086814536743654189300009119758755268914366966379;
    
    uint256 constant IC49x = 9526314568908119076094700695835234651094162957250064547915913284850221245319;
    uint256 constant IC49y = 15730227618173351298009042968267859432639623057971101102966916828065737244648;
    
    uint256 constant IC50x = 6472670072568480373842053521120300996010424030794999632404679518708089287164;
    uint256 constant IC50y = 7898963833078878968945959406283052823230462442082675714594502211150645429482;
    
    uint256 constant IC51x = 802501415753211709060912762321233270946469647147322790608311162954906470408;
    uint256 constant IC51y = 13135980094289432197743779001782214277173979939771092412224177468188992468848;
    
    uint256 constant IC52x = 21070979591398676948650920448790030469733609838425960463223210652051650707496;
    uint256 constant IC52y = 10281234735261000915210256367430280912786120316895479725624018842725400902530;
    
    uint256 constant IC53x = 14991546896422690508433132510128604178213489513127904850220561745229180145054;
    uint256 constant IC53y = 12162974338577724852630225755121526907123362019295175123034769416114815268938;
    
    uint256 constant IC54x = 1176110867533708534710504635216909833889383821096280880179681218012102526624;
    uint256 constant IC54y = 21883486024186688122901630172797267459128117483919824251799287159775703290390;
    
    uint256 constant IC55x = 5676908058032031286736110652693717922582166088809445578374265061106213118734;
    uint256 constant IC55y = 10407734549322912738727532133354355280578856808258142745527928430021303407513;
    
    uint256 constant IC56x = 16865536043646454201266336820955523081908248008122167647153362292609192558795;
    uint256 constant IC56y = 1550906102081335436085231856188810400590280968454822353145666490882557825646;
    
    uint256 constant IC57x = 383058368577151369566732537514136934594319930055447561391162302388359866539;
    uint256 constant IC57y = 6097962615042051089180065289955995255928432200583425315070379263910535436758;
    
    uint256 constant IC58x = 254857736986760002924023279057122706321681181079217761896543117817771200427;
    uint256 constant IC58y = 10022840725037562277829958632845907337865440903834442982059152847497818922393;
    
    uint256 constant IC59x = 15016810736280549252135271829308899840552295766954527483883255916214347695817;
    uint256 constant IC59y = 19451069086359684069914973539936463075755248799279353736488984719593384570949;
    
    uint256 constant IC60x = 440503925585020894265474518647442941502387595570432232579858494548697562812;
    uint256 constant IC60y = 10777908960493736184295894564129404889481074860004661897376835543199537792594;
    
    uint256 constant IC61x = 3377805344992616675474165189191374464813605957487019933941329268757115266266;
    uint256 constant IC61y = 14680900875135544942091605453556252669506994071432481866269391842231227954929;
    
    uint256 constant IC62x = 2403797889195637687533023279749807270094969472605600888400017214735558557992;
    uint256 constant IC62y = 1950383983936033847187481448946798337832971998695156633746159435543708664333;
    
    uint256 constant IC63x = 18187675948268736058283073248575021452667687051678233875447310697948260717573;
    uint256 constant IC63y = 3719868711044441444254765760616140687571323528133112129469319637604262669357;
    
    uint256 constant IC64x = 7258068868433922786646484766517408249247233066643238109107245390243177373782;
    uint256 constant IC64y = 7711522577398861029787463525168861071968707081458270744372727267061203148582;
    
    uint256 constant IC65x = 992299110068861589455958787875116447184880371942267309498136801530742447733;
    uint256 constant IC65y = 15181124709508761186376178648237367706073490373249946080975324671527718683108;
    
    uint256 constant IC66x = 19426033643711664689180559265605545854966569190128280418366678183527513514097;
    uint256 constant IC66y = 11660439514873136835783241666877982955022460971048982231457406763799979567537;
    
    uint256 constant IC67x = 18455185483774787882564360858095744018339423817045148938547546075707422654520;
    uint256 constant IC67y = 1474128022150631812705997073839243536151589432022280619983390979344552155959;
    
    uint256 constant IC68x = 14553172854269831197605850468268883401303414136800318781367725170343465094961;
    uint256 constant IC68y = 18670077951895445830513085328120066911530684778665916111122360242063152518630;
    
    uint256 constant IC69x = 17283882001265217757669912023718218616104218732542739799809394619821407403106;
    uint256 constant IC69y = 7274881838728674441537803703557802324126363785578226803022150270053203276081;
    
    uint256 constant IC70x = 811218026327923540133671727811827559434089989076393868343447862140809420402;
    uint256 constant IC70y = 9086338468476210031524605661709591120304245589706897372564584785716987136994;
    
    uint256 constant IC71x = 16016767258292003234757299393614361315016871244493569813572599062531762610402;
    uint256 constant IC71y = 18197653336157087165371201414162552184663478956441080734932716515538462895157;
    
    uint256 constant IC72x = 7693377437693152208884317991554656314553385873054783063390836428260705304633;
    uint256 constant IC72y = 14276135984705143768049441549568906090809554306163502454151846284356311780724;
    
    uint256 constant IC73x = 13734809961092779402956765878585666255502865634078319784549075882412812279112;
    uint256 constant IC73y = 19955455330953185588212902219632759570033119895312122262765604820352716970797;
    
    uint256 constant IC74x = 14484181069148608545420418794329186320104572471279046358664297054077373017320;
    uint256 constant IC74y = 2247864311348645794237346758705768469942976211966792620131804569737953030411;
    
    uint256 constant IC75x = 17509562837342577054672925331038221572973857569832586331032378936592003801581;
    uint256 constant IC75y = 15893380885071363215858220427026319726787909080838206095250448682590495481697;
    
    uint256 constant IC76x = 15250129201153862393118794885454906672026609476805436590444572596839200391343;
    uint256 constant IC76y = 20413798000294732630547600787354236834967045963376610112145364621609076155482;
    
    uint256 constant IC77x = 12854502106112026924619810496765433029794707255575019587502612237675750002204;
    uint256 constant IC77y = 18934641436227826059075192920675965756729956653046647453525977116357326826897;
    
    uint256 constant IC78x = 3145937923274828190060585308271639379341991963218988331840009385027640095543;
    uint256 constant IC78y = 12390881864536006861225331934818670605943214321429494490364981109348264037626;
    
 
    // Memory data
    uint16 constant pVk = 0;
    uint16 constant pPairing = 128;

    uint16 constant pLastMem = 896;

    function verifyProof(uint[2] calldata _pA, uint[2][2] calldata _pB, uint[2] calldata _pC, uint[78] calldata _pubSignals) public view returns (bool) {
        assembly {
            function checkField(v) {
                if iszero(lt(v, q)) {
                    mstore(0, 0)
                    return(0, 0x20)
                }
            }
            
            // G1 function to multiply a G1 value(x,y) to value in an address
            function g1_mulAccC(pR, x, y, s) {
                let success
                let mIn := mload(0x40)
                mstore(mIn, x)
                mstore(add(mIn, 32), y)
                mstore(add(mIn, 64), s)

                success := staticcall(sub(gas(), 2000), 7, mIn, 96, mIn, 64)

                if iszero(success) {
                    mstore(0, 0)
                    return(0, 0x20)
                }

                mstore(add(mIn, 64), mload(pR))
                mstore(add(mIn, 96), mload(add(pR, 32)))

                success := staticcall(sub(gas(), 2000), 6, mIn, 128, pR, 64)

                if iszero(success) {
                    mstore(0, 0)
                    return(0, 0x20)
                }
            }

            function checkPairing(pA, pB, pC, pubSignals, pMem) -> isOk {
                let _pPairing := add(pMem, pPairing)
                let _pVk := add(pMem, pVk)

                mstore(_pVk, IC0x)
                mstore(add(_pVk, 32), IC0y)

                // Compute the linear combination vk_x
                
                g1_mulAccC(_pVk, IC1x, IC1y, calldataload(add(pubSignals, 0)))
                
                g1_mulAccC(_pVk, IC2x, IC2y, calldataload(add(pubSignals, 32)))
                
                g1_mulAccC(_pVk, IC3x, IC3y, calldataload(add(pubSignals, 64)))
                
                g1_mulAccC(_pVk, IC4x, IC4y, calldataload(add(pubSignals, 96)))
                
                g1_mulAccC(_pVk, IC5x, IC5y, calldataload(add(pubSignals, 128)))
                
                g1_mulAccC(_pVk, IC6x, IC6y, calldataload(add(pubSignals, 160)))
                
                g1_mulAccC(_pVk, IC7x, IC7y, calldataload(add(pubSignals, 192)))
                
                g1_mulAccC(_pVk, IC8x, IC8y, calldataload(add(pubSignals, 224)))
                
                g1_mulAccC(_pVk, IC9x, IC9y, calldataload(add(pubSignals, 256)))
                
                g1_mulAccC(_pVk, IC10x, IC10y, calldataload(add(pubSignals, 288)))
                
                g1_mulAccC(_pVk, IC11x, IC11y, calldataload(add(pubSignals, 320)))
                
                g1_mulAccC(_pVk, IC12x, IC12y, calldataload(add(pubSignals, 352)))
                
                g1_mulAccC(_pVk, IC13x, IC13y, calldataload(add(pubSignals, 384)))
                
                g1_mulAccC(_pVk, IC14x, IC14y, calldataload(add(pubSignals, 416)))
                
                g1_mulAccC(_pVk, IC15x, IC15y, calldataload(add(pubSignals, 448)))
                
                g1_mulAccC(_pVk, IC16x, IC16y, calldataload(add(pubSignals, 480)))
                
                g1_mulAccC(_pVk, IC17x, IC17y, calldataload(add(pubSignals, 512)))
                
                g1_mulAccC(_pVk, IC18x, IC18y, calldataload(add(pubSignals, 544)))
                
                g1_mulAccC(_pVk, IC19x, IC19y, calldataload(add(pubSignals, 576)))
                
                g1_mulAccC(_pVk, IC20x, IC20y, calldataload(add(pubSignals, 608)))
                
                g1_mulAccC(_pVk, IC21x, IC21y, calldataload(add(pubSignals, 640)))
                
                g1_mulAccC(_pVk, IC22x, IC22y, calldataload(add(pubSignals, 672)))
                
                g1_mulAccC(_pVk, IC23x, IC23y, calldataload(add(pubSignals, 704)))
                
                g1_mulAccC(_pVk, IC24x, IC24y, calldataload(add(pubSignals, 736)))
                
                g1_mulAccC(_pVk, IC25x, IC25y, calldataload(add(pubSignals, 768)))
                
                g1_mulAccC(_pVk, IC26x, IC26y, calldataload(add(pubSignals, 800)))
                
                g1_mulAccC(_pVk, IC27x, IC27y, calldataload(add(pubSignals, 832)))
                
                g1_mulAccC(_pVk, IC28x, IC28y, calldataload(add(pubSignals, 864)))
                
                g1_mulAccC(_pVk, IC29x, IC29y, calldataload(add(pubSignals, 896)))
                
                g1_mulAccC(_pVk, IC30x, IC30y, calldataload(add(pubSignals, 928)))
                
                g1_mulAccC(_pVk, IC31x, IC31y, calldataload(add(pubSignals, 960)))
                
                g1_mulAccC(_pVk, IC32x, IC32y, calldataload(add(pubSignals, 992)))
                
                g1_mulAccC(_pVk, IC33x, IC33y, calldataload(add(pubSignals, 1024)))
                
                g1_mulAccC(_pVk, IC34x, IC34y, calldataload(add(pubSignals, 1056)))
                
                g1_mulAccC(_pVk, IC35x, IC35y, calldataload(add(pubSignals, 1088)))
                
                g1_mulAccC(_pVk, IC36x, IC36y, calldataload(add(pubSignals, 1120)))
                
                g1_mulAccC(_pVk, IC37x, IC37y, calldataload(add(pubSignals, 1152)))
                
                g1_mulAccC(_pVk, IC38x, IC38y, calldataload(add(pubSignals, 1184)))
                
                g1_mulAccC(_pVk, IC39x, IC39y, calldataload(add(pubSignals, 1216)))
                
                g1_mulAccC(_pVk, IC40x, IC40y, calldataload(add(pubSignals, 1248)))
                
                g1_mulAccC(_pVk, IC41x, IC41y, calldataload(add(pubSignals, 1280)))
                
                g1_mulAccC(_pVk, IC42x, IC42y, calldataload(add(pubSignals, 1312)))
                
                g1_mulAccC(_pVk, IC43x, IC43y, calldataload(add(pubSignals, 1344)))
                
                g1_mulAccC(_pVk, IC44x, IC44y, calldataload(add(pubSignals, 1376)))
                
                g1_mulAccC(_pVk, IC45x, IC45y, calldataload(add(pubSignals, 1408)))
                
                g1_mulAccC(_pVk, IC46x, IC46y, calldataload(add(pubSignals, 1440)))
                
                g1_mulAccC(_pVk, IC47x, IC47y, calldataload(add(pubSignals, 1472)))
                
                g1_mulAccC(_pVk, IC48x, IC48y, calldataload(add(pubSignals, 1504)))
                
                g1_mulAccC(_pVk, IC49x, IC49y, calldataload(add(pubSignals, 1536)))
                
                g1_mulAccC(_pVk, IC50x, IC50y, calldataload(add(pubSignals, 1568)))
                
                g1_mulAccC(_pVk, IC51x, IC51y, calldataload(add(pubSignals, 1600)))
                
                g1_mulAccC(_pVk, IC52x, IC52y, calldataload(add(pubSignals, 1632)))
                
                g1_mulAccC(_pVk, IC53x, IC53y, calldataload(add(pubSignals, 1664)))
                
                g1_mulAccC(_pVk, IC54x, IC54y, calldataload(add(pubSignals, 1696)))
                
                g1_mulAccC(_pVk, IC55x, IC55y, calldataload(add(pubSignals, 1728)))
                
                g1_mulAccC(_pVk, IC56x, IC56y, calldataload(add(pubSignals, 1760)))
                
                g1_mulAccC(_pVk, IC57x, IC57y, calldataload(add(pubSignals, 1792)))
                
                g1_mulAccC(_pVk, IC58x, IC58y, calldataload(add(pubSignals, 1824)))
                
                g1_mulAccC(_pVk, IC59x, IC59y, calldataload(add(pubSignals, 1856)))
                
                g1_mulAccC(_pVk, IC60x, IC60y, calldataload(add(pubSignals, 1888)))
                
                g1_mulAccC(_pVk, IC61x, IC61y, calldataload(add(pubSignals, 1920)))
                
                g1_mulAccC(_pVk, IC62x, IC62y, calldataload(add(pubSignals, 1952)))
                
                g1_mulAccC(_pVk, IC63x, IC63y, calldataload(add(pubSignals, 1984)))
                
                g1_mulAccC(_pVk, IC64x, IC64y, calldataload(add(pubSignals, 2016)))
                
                g1_mulAccC(_pVk, IC65x, IC65y, calldataload(add(pubSignals, 2048)))
                
                g1_mulAccC(_pVk, IC66x, IC66y, calldataload(add(pubSignals, 2080)))
                
                g1_mulAccC(_pVk, IC67x, IC67y, calldataload(add(pubSignals, 2112)))
                
                g1_mulAccC(_pVk, IC68x, IC68y, calldataload(add(pubSignals, 2144)))
                
                g1_mulAccC(_pVk, IC69x, IC69y, calldataload(add(pubSignals, 2176)))
                
                g1_mulAccC(_pVk, IC70x, IC70y, calldataload(add(pubSignals, 2208)))
                
                g1_mulAccC(_pVk, IC71x, IC71y, calldataload(add(pubSignals, 2240)))
                
                g1_mulAccC(_pVk, IC72x, IC72y, calldataload(add(pubSignals, 2272)))
                
                g1_mulAccC(_pVk, IC73x, IC73y, calldataload(add(pubSignals, 2304)))
                
                g1_mulAccC(_pVk, IC74x, IC74y, calldataload(add(pubSignals, 2336)))
                
                g1_mulAccC(_pVk, IC75x, IC75y, calldataload(add(pubSignals, 2368)))
                
                g1_mulAccC(_pVk, IC76x, IC76y, calldataload(add(pubSignals, 2400)))
                
                g1_mulAccC(_pVk, IC77x, IC77y, calldataload(add(pubSignals, 2432)))
                
                g1_mulAccC(_pVk, IC78x, IC78y, calldataload(add(pubSignals, 2464)))
                

                // -A
                mstore(_pPairing, calldataload(pA))
                mstore(add(_pPairing, 32), mod(sub(q, calldataload(add(pA, 32))), q))

                // B
                mstore(add(_pPairing, 64), calldataload(pB))
                mstore(add(_pPairing, 96), calldataload(add(pB, 32)))
                mstore(add(_pPairing, 128), calldataload(add(pB, 64)))
                mstore(add(_pPairing, 160), calldataload(add(pB, 96)))

                // alpha1
                mstore(add(_pPairing, 192), alphax)
                mstore(add(_pPairing, 224), alphay)

                // beta2
                mstore(add(_pPairing, 256), betax1)
                mstore(add(_pPairing, 288), betax2)
                mstore(add(_pPairing, 320), betay1)
                mstore(add(_pPairing, 352), betay2)

                // vk_x
                mstore(add(_pPairing, 384), mload(add(pMem, pVk)))
                mstore(add(_pPairing, 416), mload(add(pMem, add(pVk, 32))))


                // gamma2
                mstore(add(_pPairing, 448), gammax1)
                mstore(add(_pPairing, 480), gammax2)
                mstore(add(_pPairing, 512), gammay1)
                mstore(add(_pPairing, 544), gammay2)

                // C
                mstore(add(_pPairing, 576), calldataload(pC))
                mstore(add(_pPairing, 608), calldataload(add(pC, 32)))

                // delta2
                mstore(add(_pPairing, 640), deltax1)
                mstore(add(_pPairing, 672), deltax2)
                mstore(add(_pPairing, 704), deltay1)
                mstore(add(_pPairing, 736), deltay2)


                let success := staticcall(sub(gas(), 2000), 8, _pPairing, 768, _pPairing, 0x20)

                isOk := and(success, mload(_pPairing))
            }

            let pMem := mload(0x40)
            mstore(0x40, add(pMem, pLastMem))

            // Validate that all evaluations ∈ F
            
            checkField(calldataload(add(_pubSignals, 0)))
            
            checkField(calldataload(add(_pubSignals, 32)))
            
            checkField(calldataload(add(_pubSignals, 64)))
            
            checkField(calldataload(add(_pubSignals, 96)))
            
            checkField(calldataload(add(_pubSignals, 128)))
            
            checkField(calldataload(add(_pubSignals, 160)))
            
            checkField(calldataload(add(_pubSignals, 192)))
            
            checkField(calldataload(add(_pubSignals, 224)))
            
            checkField(calldataload(add(_pubSignals, 256)))
            
            checkField(calldataload(add(_pubSignals, 288)))
            
            checkField(calldataload(add(_pubSignals, 320)))
            
            checkField(calldataload(add(_pubSignals, 352)))
            
            checkField(calldataload(add(_pubSignals, 384)))
            
            checkField(calldataload(add(_pubSignals, 416)))
            
            checkField(calldataload(add(_pubSignals, 448)))
            
            checkField(calldataload(add(_pubSignals, 480)))
            
            checkField(calldataload(add(_pubSignals, 512)))
            
            checkField(calldataload(add(_pubSignals, 544)))
            
            checkField(calldataload(add(_pubSignals, 576)))
            
            checkField(calldataload(add(_pubSignals, 608)))
            
            checkField(calldataload(add(_pubSignals, 640)))
            
            checkField(calldataload(add(_pubSignals, 672)))
            
            checkField(calldataload(add(_pubSignals, 704)))
            
            checkField(calldataload(add(_pubSignals, 736)))
            
            checkField(calldataload(add(_pubSignals, 768)))
            
            checkField(calldataload(add(_pubSignals, 800)))
            
            checkField(calldataload(add(_pubSignals, 832)))
            
            checkField(calldataload(add(_pubSignals, 864)))
            
            checkField(calldataload(add(_pubSignals, 896)))
            
            checkField(calldataload(add(_pubSignals, 928)))
            
            checkField(calldataload(add(_pubSignals, 960)))
            
            checkField(calldataload(add(_pubSignals, 992)))
            
            checkField(calldataload(add(_pubSignals, 1024)))
            
            checkField(calldataload(add(_pubSignals, 1056)))
            
            checkField(calldataload(add(_pubSignals, 1088)))
            
            checkField(calldataload(add(_pubSignals, 1120)))
            
            checkField(calldataload(add(_pubSignals, 1152)))
            
            checkField(calldataload(add(_pubSignals, 1184)))
            
            checkField(calldataload(add(_pubSignals, 1216)))
            
            checkField(calldataload(add(_pubSignals, 1248)))
            
            checkField(calldataload(add(_pubSignals, 1280)))
            
            checkField(calldataload(add(_pubSignals, 1312)))
            
            checkField(calldataload(add(_pubSignals, 1344)))
            
            checkField(calldataload(add(_pubSignals, 1376)))
            
            checkField(calldataload(add(_pubSignals, 1408)))
            
            checkField(calldataload(add(_pubSignals, 1440)))
            
            checkField(calldataload(add(_pubSignals, 1472)))
            
            checkField(calldataload(add(_pubSignals, 1504)))
            
            checkField(calldataload(add(_pubSignals, 1536)))
            
            checkField(calldataload(add(_pubSignals, 1568)))
            
            checkField(calldataload(add(_pubSignals, 1600)))
            
            checkField(calldataload(add(_pubSignals, 1632)))
            
            checkField(calldataload(add(_pubSignals, 1664)))
            
            checkField(calldataload(add(_pubSignals, 1696)))
            
            checkField(calldataload(add(_pubSignals, 1728)))
            
            checkField(calldataload(add(_pubSignals, 1760)))
            
            checkField(calldataload(add(_pubSignals, 1792)))
            
            checkField(calldataload(add(_pubSignals, 1824)))
            
            checkField(calldataload(add(_pubSignals, 1856)))
            
            checkField(calldataload(add(_pubSignals, 1888)))
            
            checkField(calldataload(add(_pubSignals, 1920)))
            
            checkField(calldataload(add(_pubSignals, 1952)))
            
            checkField(calldataload(add(_pubSignals, 1984)))
            
            checkField(calldataload(add(_pubSignals, 2016)))
            
            checkField(calldataload(add(_pubSignals, 2048)))
            
            checkField(calldataload(add(_pubSignals, 2080)))
            
            checkField(calldataload(add(_pubSignals, 2112)))
            
            checkField(calldataload(add(_pubSignals, 2144)))
            
            checkField(calldataload(add(_pubSignals, 2176)))
            
            checkField(calldataload(add(_pubSignals, 2208)))
            
            checkField(calldataload(add(_pubSignals, 2240)))
            
            checkField(calldataload(add(_pubSignals, 2272)))
            
            checkField(calldataload(add(_pubSignals, 2304)))
            
            checkField(calldataload(add(_pubSignals, 2336)))
            
            checkField(calldataload(add(_pubSignals, 2368)))
            
            checkField(calldataload(add(_pubSignals, 2400)))
            
            checkField(calldataload(add(_pubSignals, 2432)))
            
            checkField(calldataload(add(_pubSignals, 2464)))
            
            checkField(calldataload(add(_pubSignals, 2496)))
            

            // Validate all evaluations
            let isValid := checkPairing(_pA, _pB, _pC, _pubSignals, pMem)

            mstore(0, isValid)
             return(0, 0x20)
         }
     }
 }
