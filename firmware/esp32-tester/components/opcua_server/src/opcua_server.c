#include "opcua_server.h"
#include "esp_log.h"
#include "board.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lwip/sockets.h"
#include "lwip/netdb.h"
#include "lwip/inet.h"
#include <string.h>
#include <stdlib.h>
#include <errno.h>

static const char *TAG = "opcua_server";

#define TCP_PORT CONFIG_OPCUA_SERVER_PORT
#define MAX_CLIENTS 3
#define RX_BUFFER_SIZE 2048
#define TX_BUFFER_SIZE 2048

/* OPC UA Node IDs */
#define NS0 0

/* Node definitions */
typedef struct {
    uint32_t node_id;
    const char *name;
    uint8_t  type;       /* 0=UInt32, 1=Double, 2=Boolean, 3=UInt16 */
    uint32_t val_u32;
    double   val_dbl;
    bool     val_bool;
    uint16_t val_u16;
} opcua_node_t;

/* Nodes: Counter(1), Temperature(2), Status(3), Relay1-6(4-9), Buzzer(10), RGB_R(11), RGB_G(12), RGB_B(13) */
#define NUM_NODES 13
static opcua_node_t nodes[NUM_NODES];
static bool running = false;

/* ---- Little-endian helpers ---- */
static void put_u16(uint8_t *p, uint16_t v) { p[0] = v & 0xFF; p[1] = (v >> 8) & 0xFF; }
static void put_u32(uint8_t *p, uint32_t v) { p[0]=v&0xFF; p[1]=(v>>8)&0xFF; p[2]=(v>>16)&0xFF; p[3]=(v>>24)&0xFF; }
static uint16_t get_u16(const uint8_t *p) { return (uint16_t)p[0] | ((uint16_t)p[1] << 8); }
static uint32_t get_u32(const uint8_t *p) { return (uint32_t)p[0] | ((uint32_t)p[1]<<8) | ((uint32_t)p[2]<<16) | ((uint32_t)p[3]<<24); }

/* ---- HEL / ACK ---- */
/* HEL is 8-byte header + 3 strings (ProtocolVersion, ReceiveBufSize, SendBufSize, MaxMsgSize, MaxChunkCount)
   Actually HEL body = ProtocolVersion(4) + ReceiveBufSize(4) + SendBufSize(4) + MaxMsgSize(4) + MaxChunkCount(4) = 20 bytes after header? No.
   HEL body: ProtocolVersion(u32) ReceiveBufSize(u32) SendBufSize(u32) MaxMsgSize(u32) MaxChunkCount(u32) EndpointUrl(string)
   ACK body: ProtocolVersion(u32) ReceiveBufSize(u32) SendBufSize(u32) MaxMsgSize(u32) MaxChunkCount(u32) */
static int build_ack(const uint8_t *hel, int hel_len, uint8_t *ack) {
    (void)hel; (void)hel_len;
    /* ACK header: "ACK" + 0x46 + reserved(u8) + msg_size(u32) */
    ack[0] = 'A'; ack[1] = 'C'; ack[2] = 'K';
    ack[3] = 0x46; /* error field, 0 = no error */
    /* body: 5 x uint32 = 20 bytes, total = 8 + 20 = 28 */
    put_u32(&ack[4], 28);  /* message size */
    put_u32(&ack[8],  0);   /* protocol version */
    put_u32(&ack[12], 65536); /* receive buf size */
    put_u32(&ack[16], 65536); /* send buf size */
    put_u32(&ack[20], 65536); /* max message size */
    put_u32(&ack[24], 1);   /* max chunk count */
    return 28;
}

/* ---- Secure Channel ---- */
/* OPN (OpenSecureChannel) - MSG header format:
   MessageType(3) + ChunkType(1) + MessageSize(u32) + SecureChannelId(u32) + ... */
/* We respond with a minimal OpenSecureChannelResponse */

static int build_opn_response(const uint8_t *req, int req_len, uint8_t *resp) {
    if (req_len < 20) return 0;
    /* Copy secure channel id from request */
    uint32_t sec_chan_id = get_u32(&req[8]);

    /* Build response: MSG header + SecurityHeader + SequenceHeader + Body */
    /* Simplified: OPN response uses "OPN" + 'F' + size */
    resp[0] = 'O'; resp[1] = 'P'; resp[2] = 'N'; resp[3] = 'F';

    /* Security header: SecurityPolicyUri(string) + SenderCertificate(string) + ReceiverCertificateThumbprint(string) */
    int pos = 12; /* after MessageType+ChunkType+Size+SecureChannelId */

    /* Skip past request security header to find sequence number */
    uint32_t seq_num = get_u32(&req[pos]);
    pos += 4;
    uint32_t req_id = get_u32(&req[pos]);
    pos += 4;

    /* Build response */
    int wp = 8; /* start of body after header */
    put_u32(&resp[wp], sec_chan_id); wp += 4;

    /* SecurityPolicyUri: empty string (len=-1 for null) */
    put_u32(&resp[wp], (uint32_t)-1); wp += 4; /* null string */

    /* SenderCertificate: null */
    put_u32(&resp[wp], (uint32_t)-1); wp += 4;

    /* ReceiverCertificateThumbprint: null */
    put_u32(&resp[wp], (uint32_t)-1); wp += 4;

    /* SequenceHeader: SequenceNumber(u32) + RequestId(u32) */
    put_u32(&resp[wp], seq_num + 1); wp += 4;
    put_u32(&resp[wp], req_id); wp += 4;

    /* Response body: NodeId (encoding byte + id) + ResponseHeader + parameters */
    /* Encoding byte for two-byte node ID: 0x00, id */
    resp[wp++] = 0x00; /* Two-byte node ID encoding */
    resp[wp++] = 0x02; /* OpenSecureChannelResponse = 448 => but we use numeric 2-byte for simplicity */

    /* Actually, let's use four-byte encoding for proper response type IDs */
    /* OpenSecureChannelResponse encoding = 0x01 (FourByte) */
    wp -= 2;
    resp[wp++] = 0x01; /* FourByte encoding */
    resp[wp++] = 0x01; /* namespace index 0 */
    put_u16(&resp[wp], 448); wp += 2; /* Response NodeId = 448 */

    /* ResponseHeader: Timestamp + RequestHandle + ServiceResult + ServiceDiagnostics + StringTable + AdditionalHeader */
    /* Timestamp (DateTime = Int64) */
    put_u32(&resp[wp], 0); wp += 4;
    put_u32(&resp[wp], 0); wp += 4;

    /* RequestHandle */
    put_u32(&resp[wp], req_id); wp += 4;

    /* ServiceResult (StatusCode) */
    put_u32(&resp[wp], 0); wp += 4; /* Good */

    /* ServiceDiagnostics (empty) */
    resp[wp++] = 0x00; /* encoding mask = 0 */

    /* StringTable: empty array (length = -1) */
    put_u32(&resp[wp], (uint32_t)-1); wp += 4;

    /* AdditionalHeader: empty */
    resp[wp++] = 0x00; /* encoding mask = 0 */

    /* OpenSecureChannelResponse body:
       ServerProtocolVersion(u32) + SecurityToken(ChannelId+TokenId+CreatedAt+RevisedLifetime) + ServerNonce(string) */
    put_u32(&resp[wp], 0); wp += 4; /* ServerProtocolVersion */

    /* SecurityToken */
    put_u32(&resp[wp], sec_chan_id); wp += 4;  /* ChannelId */
    put_u32(&resp[wp], 1); wp += 4;             /* TokenId */
    /* CreatedAt (DateTime) */
    put_u32(&resp[wp], 0); wp += 4;
    put_u32(&resp[wp], 0); wp += 4;
    /* RevisedLifetime */
    put_u32(&resp[wp], 3600000); wp += 4; /* 1 hour in ms */

    /* ServerNonce: empty */
    put_u32(&resp[wp], (uint32_t)-1); wp += 4;

    /* Set total message size */
    put_u32(&resp[4], wp);
    return wp;
}

/* ---- Read Service ---- */
static int build_read_response(const uint8_t *req, int req_len, uint8_t *resp) {
    /* Find the Read request details in the MSG body */
    /* MSG format: MSG header(8) + SecureChannelId(4) + TokenId(4) + SequenceNumber(4) + RequestId(4)
       + NodeId(1+) + RequestHeader + MaxAge + TimestampsToReturn + NodesToRead */

    /* For simplicity, scan the request for a numeric node ID pattern */
    int body_start = 24; /* approximate: 8+4+4+4+4 */

    /* Skip response node ID (we write it) */
    int wp = 8; /* after MSG header */
    put_u32(&resp[wp], get_u32(&req[8])); wp += 4;  /* SecureChannelId */
    put_u32(&resp[wp], get_u32(&req[12])); wp += 4; /* TokenId */
    put_u32(&resp[wp], get_u32(&req[16]) + 1); wp += 4; /* SequenceNumber */
    put_u32(&resp[wp], get_u32(&req[20])); wp += 4; /* RequestId */

    /* Response NodeId: ReadResponse = 705, four-byte encoding */
    resp[wp++] = 0x01;
    resp[wp++] = 0x00;
    put_u16(&resp[wp], 705); wp += 2;

    /* ResponseHeader */
    put_u32(&resp[wp], 0); wp += 4; put_u32(&resp[wp], 0); wp += 4; /* Timestamp */
    put_u32(&resp[wp], get_u32(&req[body_start + 4])); wp += 4; /* RequestHandle */
    put_u32(&resp[wp], 0); wp += 4; /* ServiceResult = Good */
    resp[wp++] = 0x00; /* Diagnostics empty */
    put_u32(&resp[wp], (uint32_t)-1); wp += 4; /* StringTable empty */
    resp[wp++] = 0x00; /* AdditionalHeader empty */

    /* Read response body: Results array */
    /* Find how many nodes to read - scan for it in request */
    /* For simplicity, we'll read through the request body to find the node list */
    /* Request body after Read NodeId: MaxAge(8) + TimestampsToReturn(4) + NodesToRead array */

    int rp = body_start;
    /* Skip request NodeId */
    if (req[rp] == 0x00) { rp += 2; }      /* TwoByte */
    else if (req[rp] == 0x01) { rp += 4; }  /* FourByte */
    else { rp += 1 + 2 + 2; }               /* Numeric */

    /* Skip RequestHeader: Timestamp(8) + RequestHandle(4) + ReturnDiagnostics(4) +
       AuditEntryId(string) + TimeoutHint(4) + AdditionalHeader */
    rp += 8 + 4 + 4; /* Timestamp + RequestHandle + ReturnDiagnostics */
    /* AuditEntryId string */
    if (rp < req_len) { int32_t sl = (int32_t)get_u32(&req[rp]); rp += 4; if (sl > 0) rp += sl; }
    /* TimeoutHint */
    rp += 4;
    /* AdditionalHeader */
    if (rp < req_len) { if (req[rp] == 0x00) rp += 1; }

    /* MaxAge (Double = 8 bytes) */
    rp += 8;
    /* TimestampsToReturn (enum u32) */
    rp += 4;

    /* NodesToRead array */
    int32_t num_nodes = 0;
    if (rp + 4 <= req_len) {
        num_nodes = (int32_t)get_u32(&req[rp]); rp += 4;
    }
    if (num_nodes <= 0) num_nodes = 0;
    if (num_nodes > 3) num_nodes = 3;

    /* Results array length */
    put_u32(&resp[wp], num_nodes); wp += 4;

    /* For each node, try to parse and return data */
    for (int n = 0; n < num_nodes && rp + 2 <= req_len; n++) {
        uint32_t target_node = 0;

        /* Parse ReadValueId: NodeId + AttributeId + ... */
        uint8_t enc = req[rp++];
        if (enc == 0x00) {
            /* TwoByte: just 1 byte id */
            target_node = req[rp++];
        } else if (enc == 0x01) {
            /* FourByte: namespace(1) + id(2) */
            rp++; /* namespace */
            target_node = get_u16(&req[rp]); rp += 2;
        } else if (enc == 0x02) {
            /* Numeric: namespace(2) + id(4) */
            rp += 2; /* namespace */
            target_node = get_u32(&req[rp]); rp += 4;
        } else {
            /* String or other - skip */
            if (rp + 4 <= req_len) {
                int32_t sl = (int32_t)get_u32(&req[rp]); rp += 4;
                if (sl > 0) rp += sl;
            }
        }
        /* Skip AttributeId(2) + NumericRange(string) + DataEncoding(QualifiedName) */
        rp += 2; /* AttributeId */
        /* NumericRange string */
        if (rp + 4 <= req_len) {
            int32_t sl = (int32_t)get_u32(&req[rp]); rp += 4;
            if (sl > 0) rp += sl;
        }
        /* DataEncoding QualifiedName: namespace(u16) + name(string) */
        if (rp + 2 <= req_len) { rp += 2; } /* namespace */
        if (rp + 4 <= req_len) {
            int32_t sl = (int32_t)get_u32(&req[rp]); rp += 4;
            if (sl > 0) rp += sl;
        }

        /* Find node */
        opcua_node_t *node = NULL;
        for (int i = 0; i < NUM_NODES; i++) {
            if (nodes[i].node_id == target_node) { node = &nodes[i]; break; }
        }

        /* DataValue: Encoding byte + Value + Status + SourceTimestamp + ServerTimestamp */
        if (node) {
            resp[wp++] = 0x01; /* HasValue flag */
            /* Variant: Encoding byte */
            switch (node->type) {
            case 0: /* UInt32 */
                resp[wp++] = 0x07; /* UInt32 encoding */
                put_u32(&resp[wp], node->val_u32); wp += 4;
                break;
            case 1: /* Double */
                resp[wp++] = 0x0B; /* Double encoding */
                memcpy(&resp[wp], &node->val_dbl, 8); wp += 8;
                break;
            case 2: /* Boolean */
                resp[wp++] = 0x01; /* Boolean encoding */
                resp[wp++] = node->val_bool ? 1 : 0;
                break;
            case 3: /* UInt16 */
                resp[wp++] = 0x06; /* UInt16 encoding */
                put_u16(&resp[wp], node->val_u16); wp += 2;
                break;
            }
        } else {
            /* Bad node id */
            resp[wp++] = 0x01; /* HasValue */
            resp[wp++] = 0x00; /* Null variant */
            /* We already set HasValue, so put a null */
            /* Actually, set Status to BadNodeIdUnknown */
            resp[wp - 2] = 0x05; /* HasValue | HasStatus */
            put_u32(&resp[wp], 0x80340000); wp += 4; /* BadNodeIdUnknown */
        }
    }

    /* DiagnosticInfos array: empty */
    put_u32(&resp[wp], (uint32_t)-1); wp += 4;

    /* Set total message size */
    resp[0] = 'M'; resp[1] = 'S'; resp[2] = 'G'; resp[3] = 'F';
    put_u32(&resp[4], wp);
    return wp;
}

/* ---- Write Service ---- */
static int build_write_response(const uint8_t *req, int req_len, uint8_t *resp) {
    int wp = 8;
    put_u32(&resp[wp], get_u32(&req[8])); wp += 4;  /* SecureChannelId */
    put_u32(&resp[wp], get_u32(&req[12])); wp += 4; /* TokenId */
    put_u32(&resp[wp], get_u32(&req[16]) + 1); wp += 4; /* SequenceNumber */
    put_u32(&resp[wp], get_u32(&req[20])); wp += 4; /* RequestId */

    /* WriteResponse = 673 */
    resp[wp++] = 0x01;
    resp[wp++] = 0x00;
    put_u16(&resp[wp], 673); wp += 2;

    /* ResponseHeader */
    put_u32(&resp[wp], 0); wp += 4; put_u32(&resp[wp], 0); wp += 4; /* Timestamp */
    put_u32(&resp[wp], 0); wp += 4; /* RequestHandle - simplified */
    put_u32(&resp[wp], 0); wp += 4; /* ServiceResult = Good */
    resp[wp++] = 0x00; /* Diagnostics */
    put_u32(&resp[wp], (uint32_t)-1); wp += 4; /* StringTable */
    resp[wp++] = 0x00; /* AdditionalHeader */

    /* Parse request to find nodes to write */
    int rp = 24;
    if (rp >= req_len) goto done;

    /* Skip request NodeId */
    {
        uint8_t enc = req[rp++];
        if (enc == 0x00) rp++;
        else if (enc == 0x01) rp += 3;
        else rp += 5;
    }

    /* Skip RequestHeader: Timestamp(8) + RequestHandle(4) + ReturnDiagnostics(4) */
    rp += 8 + 4 + 4;
    /* AuditEntryId string */
    if (rp < req_len) { int32_t sl = (int32_t)get_u32(&req[rp]); rp += 4; if (sl > 0) rp += sl; }
    /* TimeoutHint */
    rp += 4;
    /* AdditionalHeader */
    if (rp < req_len) { if (req[rp] == 0x00) rp += 1; }

    /* NodesToWrite array */
    int32_t num_writes = 0;
    if (rp + 4 <= req_len) {
        num_writes = (int32_t)get_u32(&req[rp]); rp += 4;
    }
    if (num_writes <= 0) num_writes = 0;
    if (num_writes > NUM_NODES) num_writes = NUM_NODES;

    /* Results array */
    put_u32(&resp[wp], num_writes); wp += 4;

    for (int n = 0; n < num_writes && rp + 2 <= req_len; n++) {
        uint32_t target_node = 0;

        /* Parse WriteValue: NodeId + AttributeId + NumericRange + DataValue */
        uint8_t enc = req[rp++];
        if (enc == 0x00) {
            target_node = req[rp++];
        } else if (enc == 0x01) {
            rp++; /* namespace */
            target_node = get_u16(&req[rp]); rp += 2;
        } else if (enc == 0x02) {
            rp += 2;
            target_node = get_u32(&req[rp]); rp += 4;
        }
        /* Skip AttributeId(2) */
        rp += 2;
        /* Skip NumericRange string */
        if (rp + 4 <= req_len) { int32_t sl = (int32_t)get_u32(&req[rp]); rp += 4; if (sl > 0) rp += sl; }

        /* DataValue: Encoding(1) + Variant */
        uint8_t dv_encoding = 0;
        if (rp < req_len) dv_encoding = req[rp++];
        if (dv_encoding & 0x01) { /* HasValue */
            if (rp < req_len) {
                uint8_t var_type = req[rp++];
                /* Find node and apply write */
                opcua_node_t *node = NULL;
                for (int i = 0; i < NUM_NODES; i++) {
                    if (nodes[i].node_id == target_node) { node = &nodes[i]; break; }
                }
                if (node) {
                    switch (var_type) {
                    case 0x01: /* Boolean */
                        if (rp < req_len) {
                            node->val_bool = req[rp++] != 0;
                            /* Apply to board for relay/buzzer nodes */
                            if (target_node >= 4 && target_node <= 9) {
                                board_set_relay((int)(target_node - 4), node->val_bool);
                            } else if (target_node == 10) {
                                board_set_buzzer(node->val_bool);
                            }
                        }
                        break;
                    case 0x06: /* UInt16 */
                        if (rp + 2 <= req_len) {
                            node->val_u16 = get_u16(&req[rp]); rp += 2;
                            /* Apply RGB values */
                            if (target_node == 11 || target_node == 12 || target_node == 13) {
                                board_set_rgb((uint8_t)nodes[11].val_u16,
                                              (uint8_t)nodes[12].val_u16,
                                              (uint8_t)nodes[13].val_u16);
                            }
                        }
                        break;
                    case 0x07: /* UInt32 */
                        if (rp + 4 <= req_len) { rp += 4; }
                        break;
                    case 0x0B: /* Double */
                        rp += 8;
                        break;
                    default:
                        break;
                    }
                }
                put_u32(&resp[wp], 0); wp += 4; /* Good */
            } else {
                put_u32(&resp[wp], 0x80340000); wp += 4; /* BadNodeIdUnknown */
            }
        } else {
            put_u32(&resp[wp], 0x80340000); wp += 4; /* Bad */
        }
        /* Skip Status, SourceTimestamp, ServerTimestamp if present */
        if (dv_encoding & 0x02) rp += 4;
        if (dv_encoding & 0x04) rp += 8;
        if (dv_encoding & 0x08) rp += 8;
    }

done:
    /* DiagnosticInfos: empty */
    put_u32(&resp[wp], (uint32_t)-1); wp += 4;

    resp[0] = 'M'; resp[1] = 'S'; resp[2] = 'G'; resp[3] = 'F';
    put_u32(&resp[4], wp);
    return wp;
}

/* ---- Browse Service ---- */
static int build_browse_response(const uint8_t *req, int req_len, uint8_t *resp) {
    int wp = 8;
    put_u32(&resp[wp], get_u32(&req[8])); wp += 4;  /* SecureChannelId */
    put_u32(&resp[wp], get_u32(&req[12])); wp += 4;  /* TokenId */
    put_u32(&resp[wp], get_u32(&req[16]) + 1); wp += 4; /* SequenceNumber */
    put_u32(&resp[wp], get_u32(&req[20])); wp += 4;  /* RequestId */

    /* BrowseResponse = 525 */
    resp[wp++] = 0x01;
    resp[wp++] = 0x00;
    put_u16(&resp[wp], 525); wp += 2;

    /* ResponseHeader */
    put_u32(&resp[wp], 0); wp += 4; put_u32(&resp[wp], 0); wp += 4; /* Timestamp */
    put_u32(&resp[wp], 0); wp += 4; /* RequestHandle - simplified */
    put_u32(&resp[wp], 0); wp += 4; /* ServiceResult = Good */
    resp[wp++] = 0x00; /* Diagnostics */
    put_u32(&resp[wp], (uint32_t)-1); wp += 4; /* StringTable */
    resp[wp++] = 0x00; /* AdditionalHeader */

    /* Results array: 1 BrowseResult */
    put_u32(&resp[wp], 1); wp += 4;

    /* BrowseResult: StatusCode + ContinuationPoint + References */
    put_u32(&resp[wp], 0); wp += 4; /* StatusCode = Good */
    /* ContinuationPoint: empty string (null) */
    put_u32(&resp[wp], (uint32_t)-1); wp += 4;

    /* References array */
    put_u32(&resp[wp], NUM_NODES); wp += 4;

    for (int i = 0; i < NUM_NODES; i++) {
        /* ReferenceDescription */
        /* ReferenceTypeId (NodeId, TwoByte) */
        resp[wp++] = 0x00;
        resp[wp++] = 0x23; /* Organizes = 35 */
        /* IsForward (Boolean) */
        resp[wp++] = 0x01; /* true */
        /* BrowseName (QualifiedName): namespace(u16) + name(string) */
        put_u16(&resp[wp], NS0); wp += 2;
        uint8_t name_len = (uint8_t)strlen(nodes[i].name);
        put_u32(&resp[wp], name_len); wp += 4;
        memcpy(&resp[wp], nodes[i].name, name_len); wp += name_len;
        /* DisplayName (LocalizedText): encoding(byte) + locale(string) + text(string) */
        resp[wp++] = 0x02; /* has text */
        put_u32(&resp[wp], name_len); wp += 4;
        memcpy(&resp[wp], nodes[i].name, name_len); wp += name_len;
        /* NodeClass (Enum u32) = 1 (Object) for variable nodes, but 2 (Variable) is more correct */
        put_u32(&resp[wp], 2); wp += 4; /* Variable */
        /* TypeDefinition (ExpandedNodeId): TwoByte */
        resp[wp++] = 0x00;
        resp[wp++] = 0x3B; /* BaseDataVariableType = 63 */
    }

    /* DiagnosticInfos: empty */
    put_u32(&resp[wp], (uint32_t)-1); wp += 4;

    resp[0] = 'M'; resp[1] = 'S'; resp[2] = 'G'; resp[3] = 'F';
    put_u32(&resp[4], wp);
    return wp;
}

/* ---- MSG handler ---- */
static int handle_msg(const uint8_t *req, int req_len, uint8_t *resp) {
    if (req_len < 24) return 0;

    /* After MSG header(8) + SecureChannelId(4) + TokenId(4) + SequenceNumber(4) + RequestId(4) = 24 */
    /* Then the service NodeId */
    int rp = 24;
    uint16_t service_id = 0;

    if (rp >= req_len) return 0;
    uint8_t enc = req[rp++];
    if (enc == 0x00) {
        service_id = req[rp++];
    } else if (enc == 0x01) {
        rp++; /* namespace */
        service_id = get_u16(&req[rp]); rp += 2;
    } else {
        return 0;
    }

    ESP_LOGI(TAG, "MSG service request: %u", service_id);

    switch (service_id) {
    case 631: /* Read */
        return build_read_response(req, req_len, resp);
    case 527: /* Browse */
        return build_browse_response(req, req_len, resp);
    case 673: /* Write */
        return build_write_response(req, req_len, resp);
    default:
        ESP_LOGW(TAG, "Unhandled OPC UA service: %u", service_id);
        return 0;
    }
}

/* ---- Client handler ---- */
static void handle_client(int client_fd) {
    uint8_t *rx_buf = malloc(RX_BUFFER_SIZE);
    uint8_t *tx_buf = malloc(TX_BUFFER_SIZE);
    if (!rx_buf || !tx_buf) {
        ESP_LOGE(TAG, "alloc failed");
        free(rx_buf); free(tx_buf);
        close(client_fd);
        return;
    }

    while (running) {
        int received = recv(client_fd, rx_buf, 8, MSG_WAITALL);
        if (received < 8) break;

        /* Parse header */
        uint32_t msg_size = get_u32(&rx_buf[4]);
        if (msg_size > RX_BUFFER_SIZE || msg_size < 8) break;

        /* Read remaining body */
        int remaining = msg_size - 8;
        if (remaining > 0) {
            int r = recv(client_fd, &rx_buf[8], remaining, MSG_WAITALL);
            if (r < remaining) break;
        }

        ESP_LOGI(TAG, "Received %c%c%c msg_size=%lu", rx_buf[0], rx_buf[1], rx_buf[2], (unsigned long)msg_size);

        int resp_len = 0;
        if (rx_buf[0] == 'H' && rx_buf[1] == 'E' && rx_buf[2] == 'L') {
            resp_len = build_ack(rx_buf, msg_size, tx_buf);
        } else if (rx_buf[0] == 'O' && rx_buf[1] == 'P' && rx_buf[2] == 'N') {
            resp_len = build_opn_response(rx_buf, msg_size, tx_buf);
        } else if (rx_buf[0] == 'C' && rx_buf[1] == 'L' && rx_buf[2] == 'O') {
            /* CloseSecureChannel - just close */
            break;
        } else if (rx_buf[0] == 'M' && rx_buf[1] == 'S' && rx_buf[2] == 'G') {
            resp_len = handle_msg(rx_buf, msg_size, tx_buf);
        }

        if (resp_len > 0) {
            send(client_fd, tx_buf, resp_len, 0);
        }
    }

    free(rx_buf);
    free(tx_buf);
    close(client_fd);
}

/* ---- Data update task ---- */
static void data_update_task(void *arg) {
    while (running) {
        /* Counter auto-increment */
        nodes[0].val_u32++;

        /* Temperature random 20-35 */
        nodes[1].val_dbl = 20.0 + ((double)(esp_random() % 15000) / 1000.0);

        /* Status toggle every 5s (5000ms / 1000ms = every 5 iterations) */
        static int toggle_cnt = 0;
        toggle_cnt++;
        if (toggle_cnt >= 5) {
            nodes[2].val_bool = !nodes[2].val_bool;
            toggle_cnt = 0;
        }

        /* Read relay states from board (nodes 4-9 = Relay1-6) */
        for (int i = 0; i < 6; i++) {
            nodes[4 + i].val_bool = board_get_relay(i);
        }

        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    vTaskDelete(NULL);
}

/* ---- TCP server task ---- */
static void tcp_server_task(void *arg) {
    struct sockaddr_in dest_addr;
    dest_addr.sin_addr.s_addr = htonl(INADDR_ANY);
    dest_addr.sin_family = AF_INET;
    dest_addr.sin_port = htons(TCP_PORT);

    int listen_fd = socket(AF_INET, SOCK_STREAM, IPPROTO_IP);
    if (listen_fd < 0) {
        ESP_LOGE(TAG, "socket create failed: errno %d", errno);
        vTaskDelete(NULL);
        return;
    }

    int opt = 1;
    setsockopt(listen_fd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));

    if (bind(listen_fd, (struct sockaddr *)&dest_addr, sizeof(dest_addr)) != 0) {
        ESP_LOGE(TAG, "bind failed: errno %d", errno);
        close(listen_fd);
        vTaskDelete(NULL);
        return;
    }

    if (listen(listen_fd, MAX_CLIENTS) != 0) {
        ESP_LOGE(TAG, "listen failed: errno %d", errno);
        close(listen_fd);
        vTaskDelete(NULL);
        return;
    }

    ESP_LOGI(TAG, "OPC UA Server listening on port %d", TCP_PORT);

    while (running) {
        struct sockaddr_in source_addr;
        socklen_t addr_len = sizeof(source_addr);
        int client_fd = accept(listen_fd, (struct sockaddr *)&source_addr, &addr_len);
        if (client_fd < 0) {
            if (running) ESP_LOGE(TAG, "accept failed: errno %d", errno);
            break;
        }
        ESP_LOGI(TAG, "client connected from %s:%d",
                 inet_ntoa(source_addr.sin_addr), ntohs(source_addr.sin_port));
        handle_client(client_fd);
    }

    close(listen_fd);
    vTaskDelete(NULL);
}

/* ---- Public API ---- */
esp_err_t opcua_server_init(void) {
    memset(nodes, 0, sizeof(nodes));

    /* Node 1: Counter */
    nodes[0].node_id = 1;
    nodes[0].name = "Counter";
    nodes[0].type = 0; /* UInt32 */
    nodes[0].val_u32 = 0;

    /* Node 2: Temperature */
    nodes[1].node_id = 2;
    nodes[1].name = "Temperature";
    nodes[1].type = 1; /* Double */
    nodes[1].val_dbl = 25.0;

    /* Node 3: Status */
    nodes[2].node_id = 3;
    nodes[2].name = "Status";
    nodes[2].type = 2; /* Boolean */
    nodes[2].val_bool = false;

    /* Nodes 4-9: Relay1-6 */
    static const char *relay_names[6] = {"Relay1", "Relay2", "Relay3", "Relay4", "Relay5", "Relay6"};
    for (int i = 0; i < 6; i++) {
        nodes[4 + i].node_id = 4 + i;
        nodes[4 + i].name = relay_names[i];
        nodes[4 + i].type = 2; /* Boolean */
        nodes[4 + i].val_bool = false;
    }

    /* Node 10: Buzzer */
    nodes[10].node_id = 10;
    nodes[10].name = "Buzzer";
    nodes[10].type = 2; /* Boolean */
    nodes[10].val_bool = false;

    /* Node 11: RGB_R */
    nodes[11].node_id = 11;
    nodes[11].name = "RGB_R";
    nodes[11].type = 3; /* UInt16 */
    nodes[11].val_u16 = 0;

    /* Node 12: RGB_G */
    nodes[12].node_id = 12;
    nodes[12].name = "RGB_G";
    nodes[12].type = 3; /* UInt16 */
    nodes[12].val_u16 = 0;

    /* Node 13: RGB_B */
    nodes[13].node_id = 13;
    nodes[13].name = "RGB_B";
    nodes[13].type = 3; /* UInt16 */
    nodes[13].val_u16 = 0;

    board_init();
    running = true;
    ESP_LOGI(TAG, "OPC UA Server initialized (port=%d)", TCP_PORT);
    return ESP_OK;
}

esp_err_t opcua_server_start(void) {
    xTaskCreate(tcp_server_task, "opcua_srv", 8192, NULL,
                tskIDLE_PRIORITY + 5, NULL);
    xTaskCreate(data_update_task, "opcua_data", 3072, NULL,
                tskIDLE_PRIORITY + 3, NULL);
    return ESP_OK;
}
