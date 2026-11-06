// Open the actual WebView context menu of a run-owned accessibility element.
// Build with the Windows SDK: cl /EHsc qa-delete-context.cpp ole32.lib oleaut32.lib
#include <windows.h>
#include <UIAutomation.h>
#include <wrl/client.h>
#include <string>
using Microsoft::WRL::ComPtr;

int wmain(int argc, wchar_t **argv) {
    if (argc != 3 || FAILED(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED))) return 1;
    HWND hwnd = reinterpret_cast<HWND>(_wcstoui64(argv[1], nullptr, 10));
    if (!IsWindow(hwnd)) return 2;
    ComPtr<IUIAutomation> automation;
    ComPtr<IUIAutomationElement> window, row;
    ComPtr<IUIAutomationCondition> match;
    ComPtr<IUIAutomationElement3> target;
    VARIANT id{};
    id.vt = VT_BSTR;
    id.bstrVal = SysAllocString((std::wstring(L"qa-fixture-") + argv[2]).c_str());
    HRESULT result = CoCreateInstance(CLSID_CUIAutomation, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&automation));
    if (SUCCEEDED(result)) result = automation->ElementFromHandle(hwnd, &window);
    if (SUCCEEDED(result)) result = automation->CreatePropertyCondition(UIA_AutomationIdPropertyId, id, &match);
    VariantClear(&id);
    if (SUCCEEDED(result)) result = window->FindFirst(TreeScope_Descendants, match.Get(), &row);
    if (FAILED(result) || !row || FAILED(row.As(&target))) return 3;
    return SUCCEEDED(target->ShowContextMenu()) ? 0 : 4;
}
