//! Registered font vendor IDs (OS/2 `achVendID`), for `check.tables.vendorId`.
//!
//! The IDs are those of the registered font vendors page of the OpenType specification
//! (Microsoft Typography), as FontBakery 1.1.0 (https://github.com/fonttools/fontbakery, Apache
//! License 2.0) caches that page in `fontbakery/data/fontbakery-microsoft-vendorlist.cache`. The
//! placeholder IDs follow FontBakery's `googlefonts/vendor_id` check. IDs are facts of the
//! registry; the registry grows, so an ID missing here may be newly registered.

/// IDs that tools write when nobody chose one.
pub const PLACEHOLDERS: &[[u8; 4]] = &[*b"UKWN", *b"ukwn", *b"PfEd", *b"PYRS"];

/// Whether `id` is registered. Shorter IDs are padded with spaces; padding with NUL bytes is
/// accepted too, as readers treat both alike.
pub fn is_registered(id: [u8; 4]) -> bool {
    let padded = id.map(|b| if b == 0 { b' ' } else { b });
    REGISTERED.binary_search(&padded).is_ok()
}

/// Whether `id` is empty (all spaces or NULs) or a known placeholder.
pub fn is_placeholder(id: [u8; 4]) -> bool {
    id.iter().all(|b| *b == b' ' || *b == 0) || PLACEHOLDERS.contains(&id)
}

// The registered IDs, space-padded and sorted by their bytes.

const REGISTERED: &[[u8; 4]] = &[
    *b"!666", *b"!ETF", *b"!YNE", *b"$pro", *b"(FT)", *b"(K) ", *b"0.IZ", *b"00F3", *b"0264", *b"0909", *b"0VDO", *b"100 ", *b"1206", *b"123 ",
    *b"1995", *b"1ASC", *b"1BOU", *b"1FAN", *b"1KTF", *b"1UKR", *b"1dot", *b"2000", *b"205 ", *b"2DLT", *b"2REB", *b"3011", *b"3103", *b"3270",
    *b"3333", *b"39BC", *b"3ip ", *b"3tp ", *b"42  ", *b"4A  ", *b"4FEB", *b"50FX", *b"53VN", *b"5PTS", *b"6666", *b"6789", *b"72PT", *b"8686",
    *b"918 ", *b"9999", *b"@   ", *b"@MT ", *b"A&S ", *b"A&TP", *b"A2  ", *b"A70 ", *b"AA  ", *b"AAA ", *b"AARN", *b"AAT ", *b"AATF", *b"ABBO",
    *b"ABBT", *b"ABC ", *b"ABCD", *b"ABCO", *b"ABCX", *b"ABEL", *b"ABOU", *b"ABTS", *b"ABYM", *b"AC  ", *b"ACNT", *b"ACT2", *b"ACUT", *b"ACW ",
    *b"ADBE", *b"ADBO", *b"ADG ", *b"AEF ", *b"AES ", *b"AFOS", *b"AFOU", *b"AGC ", *b"AGFA", *b"AGFO", *b"AGIA", *b"AID ", *b"AIFF", *b"AIYR",
    *b"AJL ", *b"AJPT", *b"AKDL", *b"AKOF", *b"AKRU", *b"AL  ", *b"ALEF", *b"ALFA", *b"ALFT", *b"ALIF", *b"ALPH", *b"ALPN", *b"ALS ", *b"ALTR",
    *b"ALTS", *b"AMB ", *b"AMPR", *b"AMUT", *b"ANBS", *b"ANDO", *b"ANDR", *b"ANGT", *b"ANRT", *b"ANSL", *b"AOP ", *b"APCX", *b"APF ", *b"APLY",
    *b"APOS", *b"APPL", *b"APTF", *b"ARAB", *b"ARAV", *b"ARBX", *b"ARCH", *b"ARCN", *b"ARFS", *b"ARGN", *b"ARIN", *b"ARM ", *b"ARMA", *b"ARPH",
    *b"ARQX", *b"ARS ", *b"ART ", *b"ARTA", *b"ARTE", *b"ARit", *b"AS  ", *b"ASL ", *b"ASMB", *b"ASSA", *b"ASYM", *b"ATEC", *b"ATF ", *b"ATF1",
    *b"ATFF", *b"ATFM", *b"ATFS", *b"ATKN", *b"ATKS", *b"ATLR", *b"ATLS", *b"ATPK", *b"ATS ", *b"ATSM", *b"ATYP", *b"AUH ", *b"AURE", *b"AUTO",
    *b"AVFF", *b"AVP ", *b"AVT ", *b"AWSM", *b"AX86", *b"AY  ", *b"AYRE", *b"AZLS", *b"Ad56", *b"Adam", *b"ArTy", *b"At  ", *b"AvGd", *b"B&H ",
    *b"BABA", *b"BAPS", *b"BARS", *b"BASE", *b"BAT ", *b"BB  ", *b"BBB ", *b"BBBB", *b"BBFF", *b"BBOX", *b"BBSL", *b"BBTF", *b"BBTL", *b"BBTY",
    *b"BCAI", *b"BCDS", *b"BCHF", *b"BCHN", *b"BCP ", *b"BD71", *b"BDFZ", *b"BDTD", *b"BDZ ", *b"BE  ", *b"BEAR", *b"BEN ", *b"BERT", *b"BEST",
    *b"BF  ", *b"BFGS", *b"BGDN", *b"BHRT", *b"BHS ", *b"BIJU", *b"BIRD", *b"BITM", *b"BITS", *b"BJHa", *b"BL  ", *b"BLAB", *b"BLAH", *b"BLCK",
    *b"BLF ", *b"BLFF", *b"BLI ", *b"BLIF", *b"BLK ", *b"BLKT", *b"BLOM", *b"BLST", *b"BLTS", *b"BLUE", *b"BLV ", *b"BLZT", *b"BM86", *b"BMD ",
    *b"BNDR", *b"BNFF", *b"BNT ", *b"BOGG", *b"BOGS", *b"BOLD", *b"BOLE", *b"BOLT", *b"BOMF", *b"BONE", *b"BOOM", *b"BORW", *b"BOUF", *b"BOYB",
    *b"BRBT", *b"BRDV", *b"BREM", *b"BRET", *b"BRIJ", *b"BROS", *b"BRTC", *b"BRTE", *b"BRWN", *b"BS  ", *b"BSBS", *b"BST ", *b"BSTD", *b"BSTF",
    *b"BSYV", *b"BTF ", *b"BTFF", *b"BTYP", *b"BUBU", *b"BUN ", *b"BVH ", *b"BWFW", *b"BWJC", *b"BZER", *b"Bela", *b"BenP", *b"Buer", *b"BwT ",
    *b"C&B ", *b"C&C ", *b"C&G ", *b"C21 ", *b"CAB ", *b"CAD3", *b"CAF ", *b"CAFE", *b"CAK ", *b"CAL ", *b"CAM ", *b"CANA", *b"CANO", *b"CAPE",
    *b"CAPS", *b"CASL", *b"CATG", *b"CAVE", *b"CB  ", *b"CBDO", *b"CBWJ", *b"CCCT", *b"CDAC", *b"CDFP", *b"CELB", *b"CENT", *b"CERC", *b"CF  ",
    *b"CFA ", *b"CFAB", *b"CFF ", *b"CHAI", *b"CHIC", *b"CHLL", *b"CHRY", *b"CHTY", *b"CINE", *b"CJCJ", *b"CJT ", *b"CKTP", *b"CLM ", *b"CLNT",
    *b"CLP ", *b"CLT ", *b"CLVR", *b"CMDX", *b"CMJK", *b"CMMA", *b"CNFA", *b"CNK ", *b"CNTP", *b"CNTY", *b"COFM", *b"COFO", *b"COLM", *b"COMI",
    *b"COMM", *b"CONR", *b"CONT", *b"COOK", *b"COOL", *b"CORD", *b"CORE", *b"COTF", *b"COTY", *b"CP  ", *b"CPTL", *b"CR8 ", *b"CROS", *b"CRRT",
    *b"CSCF", *b"CSCO", *b"CSDA", *b"CSTM", *b"CT  ", *b"CTCO", *b"CTDL", *b"CTL ", *b"CTYP", *b"CUBI", *b"CYLA", *b"CYPE", *b"CYRE", *b"CYTY",
    *b"CZOO", *b"CZYK", *b"D&ST", *b"D502", *b"DADA", *b"DAIN", *b"DAMA", *b"DARK", *b"DARN", *b"DB  ", *b"DBCO", *b"DBDS", *b"DBFF", *b"DC  ",
    *b"DD  ", *b"DDT ", *b"DELV", *b"DETF", *b"DF  ", *b"DFHA", *b"DFS ", *b"DFX ", *b"DG  ", *b"DGL ", *b"DGRY", *b"DHA ", *b"DHRM", *b"DICO",
    *b"DIFO", *b"DIMK", *b"DJDJ", *b"DKMS", *b"DKSP", *b"DMGR", *b"DNF ", *b"DO  ", *b"DOCS", *b"DOM ", *b"DONS", *b"DOT ", *b"DOUG", *b"DP  ",
    *b"DR  ", *b"DRFN", *b"DRT ", *b"DRZF", *b"DS  ", *b"DSBV", *b"DSCI", *b"DSGN", *b"DSKY", *b"DSLF", *b"DSSR", *b"DSST", *b"DST ", *b"DSTE",
    *b"DT  ", *b"DTC ", *b"DTF ", *b"DTIL", *b"DTL ", *b"DTPS", *b"DTSM", *b"DTZM", *b"DU  ", *b"DUAL", *b"DUXB", *b"DX  ", *b"DXTF", *b"DY  ",
    *b"DYNA", *b"DZRF", *b"Deco", *b"DnGr", *b"DonR", *b"EAFN", *b"EAST", *b"EB  ", *b"ECAL", *b"EDBI", *b"EDGE", *b"EDIT", *b"EDU ", *b"EF  ",
    *b"EFF ", *b"EFFD", *b"EFI ", *b"EFNT", *b"EFST", *b"EFWS", *b"EINW", *b"EKIO", *b"EKTF", *b"ELE ", *b"ELEO", *b"ELIO", *b"ELLR", *b"ELSE",
    *b"EMGR", *b"EMPH", *b"EMT ", *b"EPS ", *b"EPSN", *b"ERKO", *b"ESAD", *b"ESD ", *b"ESIG", *b"ESTF", *b"ET  ", *b"ETAL", *b"ETCO", *b"ETDF",
    *b"ETIO", *b"ETYM", *b"ETYP", *b"EVER", *b"Egg9", *b"F5  ", *b"FA  ", *b"FA37", *b"FAFO", *b"FAIR", *b"FAKT", *b"FAM ", *b"FANC", *b"FAPY",
    *b"FARH", *b"FAT ", *b"FAUX", *b"FAY ", *b"FBI ", *b"FBRC", *b"FBRO", *b"FBS ", *b"FC  ", *b"FCAB", *b"FCAN", *b"FCB ", *b"FCTP", *b"FDI ",
    *b"FEED", *b"FEOR", *b"FETZ", *b"FFBR", *b"FFF ", *b"FFFF", *b"FG  ", *b"FGOD", *b"FHF ", *b"FHTP", *b"FILI", *b"FIRA", *b"FIRE", *b"FIRM",
    *b"FJ  ", *b"FJR ", *b"FJTY", *b"FK  ", *b"FKAI", *b"FKT ", *b"FL  ", *b"FM  ", *b"FMFO", *b"FMLJ", *b"FNBD", *b"FNKI", *b"FNSA", *b"FNTF",
    *b"FNTP", *b"FOGC", *b"FONT", *b"FONZ", *b"FOR ", *b"FORM", *b"FORT", *b"FOS ", *b"FOSH", *b"FOSP", *b"FOST", *b"FOUN", *b"FP  ", *b"FPCY",
    *b"FPKY", *b"FPPL", *b"FPTF", *b"FRGT", *b"FRJN", *b"FRML", *b"FRNK", *b"FRNZ", *b"FRTH", *b"FS  ", *b"FSD ", *b"FSE ", *b"FSFS", *b"FSI ",
    *b"FSL ", *b"FSLF", *b"FSTR", *b"FT  ", *b"FTC ", *b"FTDS", *b"FTF ", *b"FTFT", *b"FTGD", *b"FTH ", *b"FTN ", *b"FTPT", *b"FUNA", *b"FVL ",
    *b"FWD ", *b"FWKS", *b"FWRE", *b"FXTL", *b"FXTP", *b"FY  ", *b"FeCo", *b"FeTy", *b"FeoN", *b"FoFa", *b"FoHa", *b"GAF ", *b"GAKU", *b"GAL ",
    *b"GALA", *b"GALO", *b"GALW", *b"GARI", *b"GATF", *b"GD  ", *b"GEEZ", *b"GF  ", *b"GFNT", *b"GFT ", *b"GGWA", *b"GIA ", *b"GIFT", *b"GIGA",
    *b"GKOU", *b"GLAD", *b"GLCF", *b"GLDN", *b"GLGX", *b"GLIN", *b"GLRY", *b"GLYF", *b"GNRL", *b"GNU ", *b"GOAT", *b"GOGO", *b"GOHE", *b"GOOG",
    *b"GPI ", *b"GRAF", *b"GRCR", *b"GREY", *b"GRIL", *b"GRIM", *b"GRPS", *b"GS  ", *b"GT  ", *b"GTF ", *b"GTL ", *b"GTYP", *b"GTff", *b"GWPL",
    *b"GXH ", *b"GYST", *b"Gco ", *b"GenT", *b"Goss", *b"H   ", *b"H&FJ", *b"HA  ", *b"HACO", *b"HAD ", *b"HAIL", *b"HAL ", *b"HALB", *b"HAMZ",
    *b"HAUS", *b"HB  ", *b"HBM ", *b"HBT ", *b"HCMP", *b"HD  ", *b"HDCO", *b"HDZP", *b"HEB ", *b"HERZ", *b"HEUM", *b"HEX ", *b"HEYP", *b"HFEW",
    *b"HFJ ", *b"HH  ", *b"HIH ", *b"HILL", *b"HJZ ", *b"HK  ", *b"HKSY", *b"HL  ", *b"HLDN", *b"HM  ", *b"HN  ", *b"HOH ", *b"HOOL", *b"HOUS",
    *b"HP  ", *b"HS  ", *b"HSH ", *b"HT  ", *b"HTA ", *b"HTCO", *b"HTF ", *b"HTT ", *b"HU  ", *b"HUY!", *b"HVD ", *b"HWT ", *b"HXTP", *b"HY  ",
    *b"HYPE", *b"HYPN", *b"HZ  ", *b"Hafo", *b"HanS", *b"HenT", *b"HoP ", *b"HoX ", *b"Hone", *b"IAMT", *b"IB  ", *b"IBCR", *b"IBM ", *b"IC  ",
    *b"IDAU", *b"IDEA", *b"IDEE", *b"IDF ", *b"IDKB", *b"IDL ", *b"IDLT", *b"IDMS", *b"IFF ", *b"IFHC", *b"IH  ", *b"IINF", *b"IKMJ", *b"IKN ",
    *b"IKOF", *b"ILP ", *b"IMPR", *b"INCD", *b"INDG", *b"INFS", *b"INGA", *b"INGT", *b"INNO", *b"INRA", *b"INTF", *b"INTP", *b"INTR", *b"INVC",
    *b"INVD", *b"INVI", *b"INVT", *b"IRRT", *b"ISAC", *b"ISE ", *b"ITC ", *b"ITF ", *b"ITFO", *b"ITSK", *b"ITSM", *b"IWA ", *b"Idt.", *b"IvyF",
    *b"J23 ", *b"JABM", *b"JAF ", *b"JAKE", *b"JAM ", *b"JAMO", *b"JANS", *b"JASO", *b"JB  ", *b"JBLT", *b"JCF ", *b"JCT ", *b"JDB ", *b"JENS",
    *b"JF  ", *b"JHA ", *b"JHC ", *b"JHF ", *b"JKOB", *b"JLCY", *b"JLIF", *b"JMN ", *b"JO M", *b"JOON", *b"JOT ", *b"JP  ", *b"JPTT", *b"JQKA",
    *b"JRW ", *b"JSTN", *b"JT  ", *b"JTD ", *b"JUST", *b"JUXT", *b"JVRT", *b"JWTM", *b"JY  ", *b"JYT ", *b"JuTy", *b"K   ", *b"K&C ", *b"KAME",
    *b"KAPT", *b"KATF", *b"KBFF", *b"KBNT", *b"KD  ", *b"KDW ", *b"KERN", *b"KEYA", *b"KF  ", *b"KFoF", *b"KGLR", *b"KH  ", *b"KHTB", *b"KHTF",
    *b"KILO", *b"KIRK", *b"KK  ", *b"KK95", *b"KLIM", *b"KLNT", *b"KLTF", *b"KMR ", *b"KMRS", *b"KNON", *b"KNST", *b"KNTR", *b"KNWT", *b"KO  ",
    *b"KOKN", *b"KOP ", *b"KORK", *b"KOST", *b"KOVL", *b"KP  ", *b"KRND", *b"KSES", *b"KT  ", *b"KTF ", *b"KTKM", *b"KTRF", *b"KTTF", *b"KUBA",
    *b"KUSH", *b"KWKK", *b"KrKo", *b"L2M3", *b"L52 ", *b"LA  ", *b"LADD", *b"LAGA", *b"LAIT", *b"LAND", *b"LANS", *b"LARA", *b"LARR", *b"LAUD",
    *b"LAYT", *b"LArt", *b"LBV ", *b"LCMT", *b"LCRM", *b"LDNT", *b"LDTP", *b"LEAF", *b"LECH", *b"LEKS", *b"LETR", *b"LETS", *b"LFS ", *b"LGFF",
    *b"LGX ", *b"LHF ", *b"LIAM", *b"LIEB", *b"LIGA", *b"LIND", *b"LING", *b"LINK", *b"LINO", *b"LINZ", *b"LIVE", *b"LJ  ", *b"LJDS", *b"LKAS",
    *b"LKTF", *b"LL75", *b"LLD ", *b"LLDL", *b"LLGW", *b"LLP ", *b"LMNT", *b"LNGU", *b"LNTO", *b"LOFF", *b"LOMT", *b"LONT", *b"LORO", *b"LOSK",
    *b"LOU ", *b"LOUD", *b"LOVE", *b"LP  ", *b"LPKL", *b"LS  ", *b"LST ", *b"LT  ", *b"LTF ", *b"LTFD", *b"LTRK", *b"LTRS", *b"LTRX", *b"LTT ",
    *b"LTTR", *b"LUD ", *b"LUDE", *b"LUFF", *b"LUSH", *b"LUV ", *b"LVRC", *b"LVSC", *b"LXGW", *b"LZS ", *b"LaG ", *b"Leah", *b"Lee ", *b"Lev ",
    *b"Linh", *b"Log9", *b"Ltrm", *b"LuAn", *b"LuFo", *b"LuRa", *b"M   ", *b"M+F ", *b"MACR", *b"MADE", *b"MADT", *b"MAMA", *b"MANC", *b"MANS",
    *b"MAPS", *b"MARA", *b"MARK", *b"MATE", *b"MATF", *b"MATS", *b"MAW ", *b"MAXI", *b"MAXN", *b"MBDS", *b"MBF ", *b"MBJH", *b"MC  ", *b"MCHL",
    *b"MCKL", *b"MCL ", *b"MCOW", *b"MCTF", *b"MD  ", *b"MDIV", *b"MDSN", *b"MEAP", *b"MEEM", *b"MEGA", *b"MEH ", *b"MEIR", *b"MESA", *b"META",
    *b"METC", *b"METZ", *b"MF  ", *b"MFCF", *b"MFNT", *b"MG  ", *b"MGD ", *b"MH  ", *b"MHED", *b"MHMD", *b"MHTY", *b"MILL", *b"MINT", *b"MISS",
    *b"MITF", *b"MJ  ", *b"MJ&D", *b"MJR ", *b"MJWK", *b"MK  ", *b"MKSD", *b"MKTP", *b"MLAG", *b"MLBU", *b"MLF ", *b"MLFM", *b"MLFN", *b"MLGC",
    *b"MLKT", *b"MLTH", *b"MMCG", *b"MMFT", *b"MMIK", *b"MMMM", *b"MMNG", *b"MNCK", *b"MNIC", *b"MNJU", *b"MNKR", *b"MNKY", *b"MNLF", *b"MODI",
    *b"MOGT", *b"MOHT", *b"MOJI", *b"MOLG", *b"MOMI", *b"MONB", *b"MONE", *b"MONI", *b"MONO", *b"MOON", *b"MORF", *b"MORN", *b"MORO", *b"MOSH",
    *b"MOT ", *b"MOTA", *b"MOTF", *b"MPTG", *b"MR  ", *b"MRF ", *b"MRMN", *b"MRSL", *b"MRSW", *b"MRV ", *b"MS  ", *b"MSCH", *b"MSCR", *b"MSE ",
    *b"MSPG", *b"MSTK", *b"MSTR", *b"MT  ", *b"MTCO", *b"MTDS", *b"MTF ", *b"MTFO", *b"MTIF", *b"MTM!", *b"MTMF", *b"MTNT", *b"MTY ", *b"MUCC",
    *b"MUK!", *b"MULT", *b"MUNC", *b"MUSA", *b"MUTF", *b"MVB ", *b"MVST", *b"MVTP", *b"MVty", *b"MW  ", *b"MY  ", *b"MYFO", *b"MYIL", *b"MYNT",
    *b"MagD", *b"N&G ", *b"NANO", *b"NASK", *b"NASR", *b"NAUM", *b"NB  ", *b"NBM ", *b"NBR ", *b"NBTF", *b"ND  ", *b"NDEF", *b"NEAT", *b"NEC ",
    *b"NEEC", *b"NERK", *b"NEUE", *b"NEW ", *b"NFMS", *b"NFSL", *b"NG  ", *b"NGMK", *b"NICE", *b"NICK", *b"NICO", *b"NIKO", *b"NIS ", *b"NITS",
    *b"NIVA", *b"NKTL", *b"NM  ", *b"NMAD", *b"NMKK", *b"NMRK", *b"NN  ", *b"NNSS", *b"NONO", *b"NOOT", *b"NOPN", *b"NORF", *b"NORT", *b"NOSE",
    *b"NOVA", *b"NP  ", *b"NPD ", *b"NRON", *b"NU  ", *b"NUFO", *b"NVTF", *b"NWGH", *b"NX  ", *b"NYCM", *b"NYPE", *b"NaN ", *b"NorB", *b"NtT ",
    *b"OCC ", *b"OCTY", *b"ODSR", *b"OFTY", *b"OGJ ", *b"OHG ", *b"OHNO", *b"OKAY", *b"OLAP", *b"OLEX", *b"OM  ", *b"OMAR", *b"OMG ", *b"OMNI",
    *b"OMNY", *b"ONES", *b"OO  ", *b"OPEN", *b"OPR ", *b"OPTM", *b"OPTO", *b"OR  ", *b"ORBI", *b"ORK1", *b"ORNG", *b"ORTY", *b"OSFC", *b"OSPF",
    *b"OST ", *b"OSTF", *b"OTF ", *b"OTLR", *b"OTT ", *b"OTTO", *b"OTTS", *b"OTYP", *b"OURT", *b"OVER", *b"OZTY", *b"Once", *b"P22 ", *b"PANA",
    *b"PAPF", *b"PARA", *b"PARK", *b"PBL ", *b"PBLC", *b"PBNG", *b"PBRC", *b"PCJ ", *b"PDWX", *b"PEAC", *b"PEAR", *b"PEBL", *b"PECI", *b"PETR",
    *b"PF  ", *b"PFRM", *b"PFV ", *b"PHO ", *b"PHTM", *b"PILZ", *b"PINT", *b"PIXL", *b"PIZZ", *b"PKDD", *b"PLAT", *b"PLAY", *b"PLN ", *b"PMAR",
    *b"PMNT", *b"PMPT", *b"POEM", *b"POI ", *b"PPPR", *b"PRCH", *b"PRFS", *b"PRGR", *b"PRLK", *b"PRNM", *b"PROD", *b"PROT", *b"PRTF", *b"PRTP",
    *b"PSD ", *b"PSG ", *b"PSIS", *b"PSTL", *b"PSTP", *b"PSY ", *b"PSYM", *b"PT  ", *b"PTF ", *b"PTHF", *b"PTMI", *b"PTS ", *b"PTYP", *b"PUBL",
    *b"PWR ", *b"PYRS", *b"PYTE", *b"Plau", *b"PoW ", *b"Prox", *b"QMSI", *b"QRAT", *b"QTYP", *b"QUAN", *b"QUE ", *b"R667", *b"R777", *b"RAGA",
    *b"RAIN", *b"RARE", *b"RARI", *b"RAVU", *b"RAWR", *b"RDGR", *b"RDMK", *b"READ", *b"REAL", *b"RENE", *b"RES ", *b"RETY", *b"REZ ", *b"RICK",
    *b"RIXF", *b"RJPS", *b"RJST", *b"RJTY", *b"RKFN", *b"RKRD", *b"RL  ", *b"RLTF", *b"RMGC", *b"RMU ", *b"RNDY", *b"RNGD", *b"ROAD", *b"ROB ",
    *b"ROBU", *b"ROHH", *b"ROMA", *b"ROST", *b"RPTR", *b"RR  ", *b"RR41", *b"RRT ", *b"RSET", *b"RSJ ", *b"RSMS", *b"RST ", *b"RSZ ", *b"RT  ",
    *b"RTG ", *b"RTYP", *b"RUDY", *b"RUNE", *b"RVLR", *b"RXBN", *b"RYHG", *b"RYOB", *b"RYT ", *b"Ra  ", *b"S4TF", *b"SAJA", *b"SALT", *b"SAMT",
    *b"SAND", *b"SAPL", *b"SATO", *b"SATY", *b"SAX ", *b"SB  ", *b"SBFY", *b"SBT ", *b"SBYA", *b"SCHF", *b"SCIN", *b"SCTO", *b"SDFF", *b"SE  ",
    *b"SEMI", *b"SERF", *b"SERP", *b"SEVY", *b"SF  ", *b"SFF ", *b"SFS ", *b"SFUN", *b"SG  ", *b"SGNL", *b"SHAM", *b"SHAN", *b"SHFP", *b"SHFT",
    *b"SHMI", *b"SHNC", *b"SHOT", *b"SHOW", *b"SHRB", *b"SHRP", *b"SHS ", *b"SHUB", *b"SHYF", *b"SIDE", *b"SIG ", *b"SIKI", *b"SIL ", *b"SIT ",
    *b"SK  ", *b"SKKL", *b"SKP ", *b"SL  ", *b"SLCH", *b"SLDE", *b"SLFN", *b"SLTP", *b"SLVK", *b"SMC ", *b"SMPJ", *b"SN  ", *b"SNTR", *b"SOBF",
    *b"SOHO", *b"SOOP", *b"SOS ", *b"SOTY", *b"SPFM", *b"SPFO", *b"SPIL", *b"SPIR", *b"SPKE", *b"SPT ", *b"SPVT", *b"SRC ", *b"SRCE", *b"SRFT",
    *b"SSFD", *b"SSTK", *b"ST  ", *b"STAI", *b"STAN", *b"STC ", *b"STCO", *b"STF ", *b"STFD", *b"STIX", *b"STJO", *b"STLF", *b"STMT", *b"STPE",
    *b"STYK", *b"STYP", *b"STyC", *b"SUNN", *b"SUNW", *b"SVTD", *b"SWEL", *b"SWFT", *b"SWSH", *b"SWTY", *b"SXRA", *b"SYDA", *b"SYN ", *b"SYRC",
    *b"SYRF", *b"SbB ", *b"ScHo", *b"Sdtp", *b"Sean", *b"SfC ", *b"Slab", *b"Stor", *b"SyDi", *b"SySt", *b"T&S ", *b"T&T ", *b"TAB ", *b"TAF ",
    *b"TAFS", *b"TAFT", *b"TB  ", *b"TBFF", *b"TBLD", *b"TBTF", *b"TC  ", *b"TCH ", *b"TCO ", *b"TD  ", *b"TDBK", *b"TDFF", *b"TDFO", *b"TDKI",
    *b"TDR ", *b"TERM", *b"TES ", *b"TF  ", *b"TF3D", *b"TFAC", *b"TFND", *b"TFTS", *b"TFWD", *b"TFc ", *b"TG  ", *b"TGD ", *b"TGHT", *b"TGIF",
    *b"TGS ", *b"THIN", *b"THNA", *b"THYH", *b"TILD", *b"TIME", *b"TIMO", *b"TIMR", *b"TINY", *b"TIPO", *b"TIRO", *b"TJ  ", *b"TJS ", *b"TKSW",
    *b"TLIN", *b"TM  ", *b"TMBR", *b"TMF ", *b"TMKL", *b"TMNL", *b"TMT ", *b"TN  ", *b"TNB ", *b"TNOF", *b"TNOZ", *b"TNSC", *b"TNTY", *b"TO  ",
    *b"TOFU", *b"TOGF", *b"TOKO", *b"TOM6", *b"TOPP", *b"TPDP", *b"TPMA", *b"TPOC", *b"TPOP", *b"TPSP", *b"TPTA", *b"TPTC", *b"TPTQ", *b"TPTR",
    *b"TR  ", *b"TRAF", *b"TREE", *b"TRIO", *b"TROL", *b"TROY", *b"TRSH", *b"TS  ", *b"TSEN", *b"TSGT", *b"TSPC", *b"TSTY", *b"TT  ", *b"TTC ",
    *b"TTF ", *b"TTG ", *b"TTNC", *b"TTOK", *b"TTTT", *b"TTY ", *b"TUDO", *b"TWK ", *b"TWTY", *b"TY  ", *b"TYBY", *b"TYCU", *b"TYFA", *b"TYFR",
    *b"TYHA", *b"TYMD", *b"TYME", *b"TYPA", *b"TYPB", *b"TYPE", *b"TYPO", *b"TYPR", *b"TYPS", *b"TYPT", *b"TYRE", *b"TYSL", *b"TYSO", *b"TYSP",
    *b"TYSY", *b"TYTA", *b"Tlab", *b"ToSp", *b"TpTp", *b"TrAn", *b"UA  ", *b"UCT ", *b"UFAS", *b"UFCB", *b"UFDN", *b"UKUK", *b"ULA ", *b"UNDS",
    *b"UNDT", *b"UNIC", *b"UNIT", *b"UNTP", *b"UNTY", *b"UPUP", *b"URW ", *b"USGC", *b"USSS", *b"UT  ", *b"UTF ", *b"UTFI", *b"VCTR", *b"VEZ ",
    *b"VILE", *b"VINT", *b"VJ  ", *b"VKP ", *b"VLKF", *b"VLNL", *b"VLQN", *b"VLRY", *b"VMT ", *b"VN  ", *b"VO  ", *b"VOG ", *b"VOL ", *b"VOLK",
    *b"VOLT", *b"VR  ", *b"VRED", *b"VROM", *b"VS  ", *b"VT  ", *b"VTC ", *b"VTF ", *b"VTP ", *b"VTYP", *b"VVD ", *b"VZWO", *b"WAFO", *b"WALA",
    *b"WASP", *b"WBHE", *b"WCYS", *b"WE  ", *b"WEBF", *b"WEBR", *b"WEI ", *b"WERK", *b"WF  ", *b"WILL", *b"WISE", *b"WISH", *b"WKND", *b"WL  ",
    *b"WM  ", *b"WNK ", *b"WORK", *b"WS  ", *b"WSCo", *b"WT  ", *b"WZGF", *b"XCON", *b"XFC ", *b"XOT ", *b"XOTH", *b"XSCR", *b"XTMS", *b"XXL ",
    *b"XYZ ", *b"Y&Y ", *b"YADS", *b"YAM ", *b"YDI ", *b"YDS ", *b"YERK", *b"YFFI", *b"YMGT", *b"YN  ", *b"YOFF", *b"YOKO", *b"YOUR", *b"YT  ",
    *b"YU  ", *b"YUKI", *b"YVES", *b"YWFT", *b"YYYY", *b"ZAKZ", *b"ZAMJ", *b"ZANE", *b"ZEN!", *b"ZERO", *b"ZFRT", *b"ZIHI", *b"ZKZN", *b"ZORN",
    *b"ZP  ", *b"ZPL ", *b"ZSFT", *b"ZTFN", *b"ZeGr", *b"aaff", *b"aide", *b"alte", *b"amcs", *b"amin", *b"anir", *b"anty", *b"athk", *b"b52 ",
    *b"bftr", *b"bgtl", *b"bizf", *b"brdy", *b"camp", *b"cdd ", *b"corm", *b"csmp", *b"cwwf", *b"dezc", *b"djr ", *b"dmol", *b"dsbn", *b"dtpT",
    *b"fbf ", *b"fntb", *b"fofm", *b"foon", *b"frag", *b"fsmi", *b"gobl", *b"grro", *b"hano", *b"it* ", *b"jeff", *b"jftw", *b"ka  ", *b"klew",
    *b"laic", *b"lewd", *b"lool", *b"mlss", *b"mnik", *b"n8  ", *b"ncnd", *b"nkan", *b"ofdn", *b"otaf", *b"pbd0", *b"plnt", *b"pstu", *b"robo",
    *b"rthy", *b"sacc", *b"sic ", *b"skz ", *b"spty", *b"ssm0", *b"szgs", *b"tanu", *b"tots", *b"xljb", *b"z01 ", *b"zeta",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_and_placeholder_ids() {
        assert!(is_registered(*b"ADBE"));
        assert!(is_registered(*b"GOOG"));
        assert!(is_registered(*b"MS  "));
        assert!(is_registered(*b"MS\0\0"));
        assert!(!is_registered(*b"UKWN"));
        assert!(is_placeholder(*b"PfEd"));
        assert!(is_placeholder(*b"    "));
        assert!(!is_placeholder(*b"ADBE"));
        assert!(REGISTERED.windows(2).all(|w| matches!(w, [a, b] if a < b)));
    }
}
