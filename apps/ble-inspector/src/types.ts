export interface BleDevice {
  id: string;
  name: string;
  rssi: number;
  address: string;
  connected: boolean;
}

export interface BleCharacteristic {
  uuid: string;
  properties: {
    read: boolean;
    write: boolean;
    write_without_response: boolean;
    notify: boolean;
    indicate: boolean;
  };
  value: string | null;
}

export interface BleService {
  uuid: string;
  is_primary: boolean;
  characteristics: BleCharacteristic[];
}

/** The backend reports -128 when a device has no RSSI yet. */
export const RSSI_UNKNOWN = -128;
